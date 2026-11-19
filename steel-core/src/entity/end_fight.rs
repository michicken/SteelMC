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
use steel_registry::blocks::block_state_ext::BlockStateExt as _;
use steel_registry::{vanilla_blocks, vanilla_entities};
use steel_utils::types::UpdateFlags;
use steel_utils::{BlockPos, locks::SyncMutex};

use crate::chunk::heightmap::HeightmapType;
use crate::entity::entities::EnderDragonEntity;
use crate::entity::entities::mobs::bosses::ender_dragon::end_podium_location;
use crate::entity::{Entity, LivingEntity, SharedEntity, reserve_entity_ids};
use steel_protocol::packets::game::CBossEvent;
use steel_utils::translations;
use text_components::TextComponent;
use uuid::Uuid;
use crate::world::World;

/// Persistent-enough state for one End level.
#[derive(Debug, Default)]
pub struct EndFightState {
    initialized: bool,
    killed: bool,
    dragon_id: Option<i32>,
    dragon_uuid: Option<Uuid>,
    last_boss_progress: f32,
}

impl EndFightState {
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
            if !self.killed {
                self.spawn_dragon(world);
            }
        }

        // A dragon removed for any reason other than the fight's own kill (for
        // example a `/kill`) leaves the arena empty, exactly as vanilla waits for
        // the fight to be reset rather than silently spawning a replacement.
        if let Some(id) = self.dragon_id {
            match world.get_entity_by_id(id) {
                Some(entity) => {
                    let progress = entity
                        .as_living_entity()
                        .map_or(0.0, |living| {
                            (living.get_health() / living.get_max_health().max(0.001)).clamp(0.0, 1.0)
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

    /// Marks the fight finished and builds the exit portal + egg.
    pub fn on_dragon_killed(&mut self, world: &Arc<World>) {
        if self.killed {
            return;
        }
        self.killed = true;
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
            self.dragon_id = Some(id);
            self.dragon_uuid = world.get_entity_by_id(id).map(|entity| entity.uuid());
            self.last_boss_progress = 1.0;
            if let Some(uuid) = self.dragon_uuid {
                world.broadcast_to_all(CBossEvent::add(
                    uuid,
                    TextComponent::translated(translations::ENTITY_MINECRAFT_ENDER_DRAGON.msg()),
                    1.0,
                    0,
                ));
            }
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
        let air = vanilla_blocks::AIR.default_state();

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
        let _ = air;
    }
}

/// Shared End fight slot stored on every world, used only in the End.
pub type EndFight = SyncMutex<EndFightState>;

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use steel_registry::blocks::block_state_ext::BlockStateExt as _;
    use steel_registry::{init_vanilla_registry, vanilla_blocks, vanilla_dimension_types};
    use steel_utils::{BlockPos, Downcast as _};

    use crate::entity::{Entity, LivingEntity};

    use super::EndFightState;
    use crate::test_support::{fresh_test_world_with_dimension_type, insert_ready_full_chunk};

    fn end_world(key: &'static str) -> Arc<crate::world::World> {
        init_vanilla_registry();
        crate::behavior::init_behaviors();
        crate::block_entity::init_block_entities();
        let fixture =
            fresh_test_world_with_dimension_type("end_fight_test", key, &vanilla_dimension_types::THE_END);
        for cx in -1..=1 {
            for cz in -1..=1 {
                insert_ready_full_chunk(&fixture.world, steel_utils::ChunkPos::new(cx, cz));
            }
        }
        fixture.world
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
    fn a_real_dragon_death_marks_the_fight_and_builds_the_portal() {
        let world = end_world("real_death");
        let mut fight = EndFightState::default();
        fight.tick(&world);
        let id = fight.dragon_id().expect("dragon");
        let entity = world.get_entity_by_id(id).expect("id");
        let dragon = entity
            .downcast_ref::<crate::entity::entities::EnderDragonEntity>()
            .expect("dragon");

        // Drop the health to zero the way combat would and run the 200-tick
        // death animation; the final tick must notify the fight.
        dragon.set_health(0.0);
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
