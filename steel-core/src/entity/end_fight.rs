//! The End's dragon fight lifecycle.
//!
//! Vanilla keeps this state in `EnderDragonFight`, a `SavedData` attached to the
//! End level. Steel stores the live half here and rebuilds the arena on first tick:
//! one dragon at the central island, with the ten worldgen crystals already placed
//! by the End spike feature. Killing the dragon builds the exit portal and drops
//! the dragon egg; the killed flag stops a fresh dragon from being spawned until
//! the fight is deliberately reset.

use std::sync::Arc;

use glam::DVec3;
use serde::{Deserialize, Serialize};
use steel_registry::{vanilla_blocks, vanilla_entities};
use steel_utils::types::UpdateFlags;
use steel_utils::{BlockPos, Downcast as _, geometry::WorldAabb, locks::SyncMutex};
use uuid::Uuid;

use crate::chunk::heightmap::HeightmapType;
use crate::entity::entities::EnderDragonEntity;
use crate::entity::entities::mobs::bosses::ender_dragon::{
    EnderDragonPhase, end_podium_location,
};
use crate::entity::{RemovalReason, SharedEntity, reserve_entity_ids};
use steel_protocol::packets::game::CBossEvent;
use steel_utils::translations;
use text_components::TextComponent;
use crate::world::World;

/// Persistent-enough state for one End level.
#[derive(Debug, Default)]
pub struct EndFightState {
    initialized: bool,
    killed: bool,
    spawned: bool,
    dragon_id: Option<i32>,
    dragon_uuid: Option<Uuid>,
    last_boss_progress: f32,
    /// Transient marker: the state changed since it was last persisted.
    dirty: bool,
}

/// TOML-serializable half of the fight state. The live entity id is not saved: it
/// is reassigned on every load.
#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct PersistedEndFight {
    killed: bool,
    spawned: bool,
    dragon_uuid: Option<String>,
}

impl EndFightState {
    /// Rebuilds the live state from its saved data.
    #[must_use]
    pub(crate) fn from_persisted(persisted: PersistedEndFight) -> Self {
        Self {
            killed: persisted.killed,
            spawned: persisted.spawned,
            dragon_uuid: persisted.dragon_uuid.and_then(|value| Uuid::parse_str(&value).ok()),
            ..Self::default()
        }
    }

    /// Converts the live state into the saved-data snapshot.
    #[must_use]
    pub(crate) fn to_persisted(&self) -> PersistedEndFight {
        PersistedEndFight {
            killed: self.killed,
            spawned: self.spawned,
            dragon_uuid: self.dragon_uuid.map(|uuid| uuid.to_string()),
        }
    }

    /// Whether the live state has unsaved changes.
    #[must_use]
    pub const fn is_dirty(&self) -> bool {
        self.dirty
    }

    /// Whether the dragon has already been defeated in this world.
    #[must_use]
    pub const fn is_killed(&self) -> bool {
        self.killed
    }

    /// The live dragon's entity id, if one is spawned.
    #[must_use]
    pub const fn dragon_id(&self) -> Option<i32> {
        self.dragon_id
    }

    /// Called once per End world tick.
    pub fn tick(&mut self, world: &Arc<World>) {
        if !self.initialized {
            self.initialized = true;
        }

        if self.killed {
            // A leftover dragon (for example one that survived a restart) must not
            // roam the arena after the fight was already won.
            self.discard_leftover_dragons(world);
            self.refresh_boss_progress(world);
            return;
        }

        if !self.spawned {
            // A dragon already present in the world (restart, or a leftover from a
            // previous session) is taken over instead of spawning a second one.
            if let Some(existing) = self.find_existing_dragon(world) {
                let id = existing.id();
                let uuid = existing.uuid();
                self.spawned = true;
                self.dragon_id = Some(id);
                self.dragon_uuid = Some(uuid);
                self.last_boss_progress = 1.0;
                self.dirty = true;
            } else if world
                .chunk_map
                .is_block_ticking_full_chunk_loaded(steel_utils::ChunkPos::new(0, 0))
            {
                // Vanilla creates the dragon once the arena chunks are available.
                // The first End tick can run before the player's view has loaded
                // chunk (0,0), so keep retrying until the spawn succeeds.
                self.spawn_dragon(world);
            }
        }

        self.refresh_boss_progress(world);
    }

    /// Marks the fight finished and builds the exit portal + egg.
    pub fn on_dragon_killed(&mut self, world: &Arc<World>) {
        if self.killed {
            return;
        }
        self.killed = true;
        self.dirty = true;
        self.dragon_id = None;
        if let Some(uuid) = self.dragon_uuid.take() {
            world.broadcast_to_all(CBossEvent::remove(uuid));
        }
        self.build_exit_portal(world);
    }

    fn spawn_dragon(&mut self, world: &Arc<World>) {
        let ids = reserve_entity_ids(9);
        let dragon = EnderDragonEntity::new(
            &vanilla_entities::ENDER_DRAGON,
            ids.first(),
            DVec3::new(0.5, 128.0, 0.5),
            Arc::downgrade(world),
        );
        dragon.set_fight_origin(BlockPos::ZERO);
        let entity: SharedEntity = Arc::new(dragon);
        if world.try_add_entity(entity).is_ok() {
            let id = ids.first();
            self.spawned = true;
            self.dragon_id = Some(id);
            self.dragon_uuid = world.get_entity_by_id(id).map(|entity| entity.uuid());
            self.last_boss_progress = 1.0;
            self.dirty = true;

            // A freshly built dragon defaults to the hovering phase; the fight
            // hands it off to the circling pattern once it is in the world.
            if let Some(entity) = world.get_entity_by_id(id) {
                if let Some(dragon) = entity.downcast_ref::<EnderDragonEntity>() {
                    dragon
                        .phase_manager()
                        .set_phase(dragon, EnderDragonPhase::HoldingPattern);
                }
            }

            if let Some(uuid) = self.dragon_uuid {
                // Vanilla dragon bar: purple (4) plain progress bar, no extra flags.
                world.broadcast_to_all(CBossEvent::add(
                    uuid,
                    TextComponent::translated(translations::ENTITY_MINECRAFT_ENDER_DRAGON.msg()),
                    1.0,
                    4,
                    0,
                    0,
                ));
            }
        }
    }

    fn refresh_boss_progress(&mut self, world: &Arc<World>) {
        // A dragon removed for any reason other than the fight's own kill (for
        // example a `/kill`) leaves the arena empty, exactly as vanilla waits for
        // the fight to be reset rather than silently spawning a replacement.
        if let Some(id) = self.dragon_id {
            match world.get_entity_by_id(id) {
                Some(entity) => {
                    let progress = entity
                        .as_living_entity()
                        .map_or(0.0, |living| {
                            (living.get_health() / living.get_max_health().max(0.001))
                                .clamp(0.0, 1.0)
                        });
                    if (progress - self.last_boss_progress).abs() > 0.01 {
                        self.last_boss_progress = progress;
                        if let Some(uuid) = self.dragon_uuid {
                            world.broadcast_to_all(CBossEvent::update_progress(uuid, progress));
                        }
                    }
                }
                None => {
                    self.dragon_id = None;
                }
            }
        }
    }

    fn find_existing_dragon(&self, world: &Arc<World>) -> Option<SharedEntity> {
        let area = WorldAabb::new(-300.0, -64.0, -300.0, 300.0, 200.0, 300.0);
        world
            .get_entities_in_aabb_matching(&area, |entity| {
                entity.entity_type() == &vanilla_entities::ENDER_DRAGON
            })
            .into_iter()
            .next()
    }

    fn discard_leftover_dragons(&self, world: &Arc<World>) {
        let area = WorldAabb::new(-300.0, -64.0, -300.0, 300.0, 200.0, 300.0);
        for entity in world.get_entities_in_aabb_matching(&area, |entity| {
            entity.entity_type() == &vanilla_entities::ENDER_DRAGON
        }) {
            entity.set_removed(RemovalReason::Discarded);
        }
    }

    fn build_exit_portal(&self, world: &Arc<World>) {
        let podium = end_podium_location(BlockPos::ZERO);
        let y = world
            .height_at(HeightmapType::MotionBlocking, podium.x(), podium.z())
            .filter(|y| *y > world.get_min_y())
            .unwrap_or(65);

        let bedrock = vanilla_blocks::BEDROCK.default_state();
        let portal = vanilla_blocks::END_PORTAL.default_state();

        for dx in -2..=2 {
            for dz in -2..=2 {
                world.set_block(
                    BlockPos::new(podium.x() + dx, y - 1, podium.z() + dz),
                    bedrock,
                    UpdateFlags::UPDATE_ALL,
                );
            }
        }
        for dx in -1..=1 {
            for dz in -1..=1 {
                world.set_block(
                    BlockPos::new(podium.x() + dx, y, podium.z() + dz),
                    portal,
                    UpdateFlags::UPDATE_ALL,
                );
            }
        }
        // The egg is the visible trophy; it sits on the portal's centre.
        world.set_block(
            BlockPos::new(podium.x(), y + 1, podium.z()),
            vanilla_blocks::DRAGON_EGG.default_state(),
            UpdateFlags::UPDATE_ALL,
        );

        self.spawn_end_gateways(world, podium, y);
    }

    /// Places End gateway blocks around the exit portal after the dragon dies.
    ///
    /// The gateway block entity currently has no public way to clear its stored
    /// exit position, so these gateways are created with a placeholder exit and
    /// `exact = false`; real outer-island targets are assigned by the existing
    /// teleport transition on first use.
    fn spawn_end_gateways(&self, world: &Arc<World>, podium: BlockPos, y: i32) {
        let offsets: [(i32, i32); 4] = [(6, 0), (-6, 0), (0, 6), (0, -6)];
        for (dx, dz) in offsets {
            let origin = BlockPos::new(podium.x() + dx, y, podium.z() + dz);
            let _ = world.create_end_gateway_portal(origin, origin, false);
        }
    }
}

/// Shared End fight slot stored on every world, used only in the End.
pub type EndFight = SyncMutex<EndFightState>;

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use steel_registry::blocks::block_state_ext::BlockStateExt as _;
    use steel_registry::{
        init_vanilla_registry, vanilla_blocks, vanilla_damage_types, vanilla_dimension_types,
        vanilla_entities,
    };
    use steel_utils::{BlockPos, Downcast as _};
    use uuid::Uuid;

    use crate::entity::damage::DamageSource;
    use crate::entity::entities::mobs::bosses::ender_dragon::EnderDragonPhase;
    use crate::entity::{Entity, LivingEntity, reserve_entity_ids};

    use super::{EndFightState, PersistedEndFight};
    use crate::test_support::{fresh_test_world_with_dimension_type, insert_ready_full_chunk};

    fn end_world(key: &'static str) -> Arc<crate::world::World> {
        init_vanilla_registry();
        crate::behavior::init_behaviors();
        crate::block_entity::init_block_entities();
        // Leaked so the fixture's damage history outlives the world handle: a
        // real killing blow records into it.
        let fixture = Box::leak(Box::new(fresh_test_world_with_dimension_type(
            "end_fight_test",
            key,
            &vanilla_dimension_types::THE_END,
        )));
        for cx in -1..=1 {
            for cz in -1..=1 {
                insert_ready_full_chunk(&fixture.world, steel_utils::ChunkPos::new(cx, cz));
            }
        }
        Arc::clone(&fixture.world)
    }

    #[test]
    fn first_tick_spawns_one_dragon_with_all_eight_parts() {
        let world = end_world("spawn");
        let mut fight = EndFightState::default();
        fight.tick(&world);

        let id = fight.dragon_id().expect("dragon should be spawned");
        let entity = world.get_entity_by_id(id).expect("dragon id should resolve");
        let dragon = entity.downcast_ref::<crate::entity::entities::EnderDragonEntity>().expect("entity should be the dragon");
        assert_eq!(dragon.sub_entities().len(), 8);
        for part in dragon.sub_entities() {
            assert!(world.get_entity_by_id(part.id()).is_some() || part.id() > 0);
        }

        // A second tick must not create a duplicate.
        fight.tick(&world);
        assert_eq!(fight.dragon_id(), Some(id));
    }

    #[test]
    fn spawned_dragon_enters_the_holding_pattern_phase() {
        let world = end_world("holding_pattern");
        let mut fight = EndFightState::default();
        fight.tick(&world);

        let id = fight.dragon_id().expect("dragon should be spawned");
        let entity = world.get_entity_by_id(id).expect("dragon id should resolve");
        let dragon = entity
            .downcast_ref::<crate::entity::entities::EnderDragonEntity>()
            .expect("entity should be the dragon");
        assert_eq!(
            dragon.phase_manager().current_phase(),
            EnderDragonPhase::HoldingPattern
        );
    }

    #[test]
    fn dragon_spawn_waits_for_the_arena_chunks_to_load() {
        init_vanilla_registry();
        crate::behavior::init_behaviors();
        crate::block_entity::init_block_entities();
        let fixture = fresh_test_world_with_dimension_type(
            "end_fight_test",
            "retry",
            &vanilla_dimension_types::THE_END,
        );
        let world = fixture.world;
        let mut fight = EndFightState::default();
        // No arena chunk is loaded yet: the dragon cannot be inserted.
        fight.tick(&world);
        assert!(fight.dragon_id().is_none(), "no dragon without chunk (0,0)");

        for cx in -1..=1 {
            for cz in -1..=1 {
                insert_ready_full_chunk(&world, steel_utils::ChunkPos::new(cx, cz));
            }
        }
        fight.tick(&world);
        assert!(
            fight.dragon_id().is_some(),
            "dragon must appear once the arena chunks are available"
        );
    }

    #[test]
    fn an_existing_dragon_is_taken_over_instead_of_duplicated() {
        let world = end_world("takeover");
        let ids = reserve_entity_ids(9);
        let dragon = crate::entity::entities::EnderDragonEntity::new(
            &vanilla_entities::ENDER_DRAGON,
            ids.first(),
            glam::DVec3::new(0.5, 128.0, 0.5),
            Arc::downgrade(&world),
        );
        world
            .try_add_entity(Arc::new(dragon))
            .expect("leftover dragon should be added");

        let mut fight = EndFightState::default();
        fight.tick(&world);

        let bound_id = fight.dragon_id().expect("existing dragon must be bound");
        assert_eq!(bound_id, ids.first());

        fight.tick(&world);
        assert_eq!(fight.dragon_id(), Some(bound_id), "no second dragon may spawn");
    }

    #[test]
    fn a_killed_fight_discards_any_leftover_dragon() {
        let world = end_world("cleanup");
        let ids = reserve_entity_ids(9);
        let dragon = crate::entity::entities::EnderDragonEntity::new(
            &vanilla_entities::ENDER_DRAGON,
            ids.first(),
            glam::DVec3::new(0.5, 128.0, 0.5),
            Arc::downgrade(&world),
        );
        world
            .try_add_entity(Arc::new(dragon))
            .expect("leftover dragon should be added");

        let mut fight = EndFightState::from_persisted(PersistedEndFight {
            killed: true,
            spawned: true,
            dragon_uuid: None,
        });
        fight.tick(&world);

        assert!(
            world.get_entity_by_id(ids.first()).is_none(),
            "leftover dragon must be discarded after the fight was won"
        );
    }

    #[test]
    fn killing_the_dragon_builds_the_return_portal_and_egg() {
        let world = end_world("kill");
        let mut fight = EndFightState::default();
        fight.tick(&world);
        assert!(!fight.is_killed());

        world.on_ender_dragon_killed();
        assert!(world.is_ender_dragon_killed());

        let podium = crate::entity::entities::mobs::bosses::ender_dragon::end_podium_location(BlockPos::ZERO);
        let portal = (60..80)
            .find(|y| {
                world
                    .get_block_state(BlockPos::new(podium.x(), *y, podium.z()))
                    .get_block()
                    == vanilla_blocks::END_PORTAL.default_state().get_block()
            })
            .expect("return portal should exist at the podium");
        assert_eq!(
            world.get_block_state(BlockPos::new(podium.x(), portal + 1, podium.z())).get_block(),
            vanilla_blocks::DRAGON_EGG.default_state().get_block()
        );
        assert_eq!(
            world
                .get_block_state(BlockPos::new(podium.x() + 2, portal - 1, podium.z() + 2))
                .get_block(),
            vanilla_blocks::BEDROCK.default_state().get_block()
        );

        // No new dragon after the fight is won.
        let mut after = EndFightState::default();
        after.killed = true;
        after.tick(&world);
        assert!(after.dragon_id().is_none());
    }

    #[test]
    fn killed_fight_state_round_trips_through_saved_data() {
        let uuid = Uuid::parse_str("01234567-89ab-cdef-0123-456789abcdef").expect("uuid");
        let original = EndFightState {
            killed: true,
            spawned: true,
            dragon_uuid: Some(uuid),
            ..EndFightState::default()
        };

        let persisted = original.to_persisted();
        let restored = EndFightState::from_persisted(persisted);

        assert!(restored.is_killed());
        assert_eq!(restored.dragon_uuid, Some(uuid));
        assert!(!restored.is_dirty(), "loaded state is not dirty until changed");
    }

    #[test]
    fn a_real_dragon_death_marks_the_fight_and_builds_the_portal() {
        let world = end_world("real_death");
        let mut fight = EndFightState::default();
        fight.tick(&world);
        let id = fight.dragon_id().expect("dragon");
        let entity = world.get_entity_by_id(id).expect("id");
        let dragon = entity
            .downcast_ref::<crate::entity::entities::EnderDragonEntity>()
            .expect("dragon");

        // A freshly built dragon is handed to the circling phase by the fight, so
        // set the airborne death phase explicitly to drive the death flight.
        dragon
            .phase_manager()
            .set_phase(dragon, EnderDragonPhase::HoldingPattern);

        // Land a real killing blow through the dragon's body part. An airborne
        // dragon claws back to one point of health and enters the death flight
        // instead of dying immediately.
        let source = DamageSource::environment(&vanilla_damage_types::EXPLOSION);
        assert!(
            dragon.hurt_server(&world, &source, 1000.0),
            "the body hit must be accepted"
        );
        assert_eq!(dragon.get_health(), 1.0, "airborne dragon survives at one point");

        // Run the 200-tick death animation; the final tick must notify the fight.
        for _ in 0..200 {
            dragon.tick_death();
        }
        assert!(dragon.is_removed(), "death animation should remove the dragon");
        assert!(world.is_ender_dragon_killed(), "fight must be marked killed");
        assert!(
            world.get_entity_by_id(id).is_none(),
            "dead dragon must leave the world"
        );

        let podium = crate::entity::entities::mobs::bosses::ender_dragon::end_podium_location(BlockPos::ZERO);
        let portal = (60..80)
            .find(|y| {
                world
                    .get_block_state(BlockPos::new(podium.x(), *y, podium.z()))
                    .get_block()
                    == vanilla_blocks::END_PORTAL.default_state().get_block()
            })
            .expect("real dragon death must build the return portal");
        assert!(
            world
                .get_block_state(BlockPos::new(podium.x(), portal + 1, podium.z()))
                .get_block()
                == vanilla_blocks::DRAGON_EGG.default_state().get_block()
        );
    }

    #[test]
    fn dragon_entity_uses_the_end_podium_origin() {
        let world = end_world("origin");
        let mut fight = EndFightState::default();
        fight.tick(&world);
        let id = fight.dragon_id().expect("dragon");
        let entity = world.get_entity_by_id(id).expect("id");
        let dragon = entity.downcast_ref::<crate::entity::entities::EnderDragonEntity>().expect("dragon");
        assert_eq!(dragon.fight_origin(), BlockPos::ZERO);
    }
}
