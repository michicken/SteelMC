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
use steel_registry::blocks::block_state_ext::BlockStateExt as _;
use steel_registry::blocks::properties::BlockStateProperties;
use steel_registry::{vanilla_blocks, vanilla_entities};
use steel_utils::random::{Random as _, legacy_random::LegacyRandom};
use steel_utils::types::UpdateFlags;
use steel_utils::{BlockPos, ChunkPos, Direction, Downcast as _, geometry::WorldAabb, locks::SyncMutex};
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
    /// The pre-generated podium is furniture, not a kill reward. Vanilla lays it
    /// down during worldgen; Steel lays it down once the four chunks around
    /// (0,0) are fully loaded, then only flips the portal blocks on kill.
    podium_placed: bool,
    /// Y level chosen for the podium, kept so activation reuses the exact same
    /// layer even after the bedrock pillar changes the heightmap.
    podium_y: Option<i32>,
    /// Delayed gateways that could not be built because their chunk was not
    /// loaded yet. Vanilla's `END_GATEWAY_DELAYED` state waits the same way.
    pending_gateways: Vec<BlockPos>,
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

        // Vanilla generates the podium during worldgen. Do the same as soon as
        // the chunks touching (0,0) are block-ticking, so the bedrock fountain
        // is already visible before the fight and killing the dragon only
        // activates its portal blocks.
        if !self.podium_placed && end_podium_chunks_loaded(world) {
            self.ensure_end_podium(world);
        }
        self.flush_pending_gateways(world);

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

    fn ensure_end_podium(&mut self, world: &Arc<World>) {
        let podium = end_podium_location(BlockPos::ZERO);
        let y = self.podium_y.unwrap_or_else(|| {
            world
                .height_at(HeightmapType::MotionBlocking, podium.x(), podium.z())
                .filter(|y| *y > world.get_min_y())
                .unwrap_or(65)
        });
        self.podium_y = Some(y);

        // Worlds won before this fix stored the egg one block above the portal.
        // Preserve it: remember where it is before the bedrock pillar is rebuilt,
        // then move it to the Vanilla pillar top (four blocks above the portal).
        let old_egg = if self.killed {
            find_dragon_egg(world, podium, y)
        } else {
            None
        };

        place_end_podium_live(world, BlockPos::new(podium.x(), y, podium.z()), self.killed);
        self.podium_placed = true;
        self.dirty = true;

        // The first Steel implementation dropped four gateways in a plus-shape
        // six blocks from the podium. Remove that exact legacy layout once, so
        // worlds won before this fix do not keep the wrong geometry.
        if self.killed {
            remove_legacy_end_gateways(world, podium, y);
        }

        if let Some(old_egg) = old_egg {
            let target = BlockPos::new(podium.x(), y + 4, podium.z());
            if old_egg != target {
                let _ = world.set_block(
                    old_egg,
                    vanilla_blocks::AIR.default_state(),
                    UpdateFlags::UPDATE_ALL,
                );
                let _ = world.set_block(
                    target,
                    vanilla_blocks::DRAGON_EGG.default_state(),
                    UpdateFlags::UPDATE_ALL,
                );
            }
        }
    }

    fn flush_pending_gateways(&mut self, world: &Arc<World>) {
        let mut index = 0;
        while index < self.pending_gateways.len() {
            let pos = self.pending_gateways[index];
            if world.create_end_gateway_portal(pos, None, false) {
                self.pending_gateways.remove(index);
                self.dirty = true;
            } else {
                index += 1;
            }
        }
    }

    fn build_exit_portal(&mut self, world: &Arc<World>) {
        let podium = end_podium_location(BlockPos::ZERO);
        let y = self
            .podium_y
            .or_else(|| {
                world
                    .height_at(HeightmapType::MotionBlocking, podium.x(), podium.z())
                    .filter(|y| *y > world.get_min_y())
            })
            .unwrap_or(65);
        self.podium_y = Some(y);
        self.podium_placed = true;

        // The podium itself already exists. Activating only replaces the inner
        // rim with End portal blocks, exactly like Vanilla's active feature.
        place_end_podium_live(world, BlockPos::new(podium.x(), y, podium.z()), true);
        // The egg sits on top of the four-block bedrock pillar.
        world.set_block(
            BlockPos::new(podium.x(), y + 4, podium.z()),
            vanilla_blocks::DRAGON_EGG.default_state(),
            UpdateFlags::UPDATE_ALL,
        );

        self.spawn_delayed_gateway(world);
    }

    /// One gateway per won fight, at one of Vanilla's twenty fixed spots on the
    /// 96-block ring around the island (Y=75), picked by world seed.
    fn spawn_delayed_gateway(&mut self, world: &Arc<World>) {
        let mut random = LegacyRandom::from_seed(world.seed() as u64);
        let index = random.next_i32_bounded(END_GATEWAY_SLOTS);
        let pos = end_gateway_position(index);
        if !world.create_end_gateway_portal(pos, None, false) {
            self.pending_gateways.push(pos);
        }
        self.dirty = true;
    }
}

const END_GATEWAY_SLOTS: i32 = 20;
const END_GATEWAY_RING_RADIUS: f64 = 96.0;
const END_GATEWAY_Y: i32 = 75;

fn end_podium_closer_than(pos: BlockPos, origin: BlockPos, distance: f64) -> bool {
    let dx = f64::from(pos.x() - origin.x());
    let dy = f64::from(pos.y() - origin.y());
    let dz = f64::from(pos.z() - origin.z());
    dx * dx + dy * dy + dz * dz < distance * distance
}

/// Live-world counterpart of Vanilla's `EndPodiumFeature.place`.
fn place_end_podium_live(world: &Arc<World>, origin: BlockPos, active: bool) {
    let bedrock = vanilla_blocks::BEDROCK.default_state();
    let end_stone = vanilla_blocks::END_STONE.default_state();
    let air = vanilla_blocks::AIR.default_state();
    let end_portal = vanilla_blocks::END_PORTAL.default_state();

    for pos in BlockPos::between_closed(origin.offset(-4, -1, -4), origin.offset(4, 32, 4)) {
        let inside_rim = end_podium_closer_than(pos, origin, 2.5);
        if !inside_rim && !end_podium_closer_than(pos, origin, 3.5) {
            continue;
        }

        let state = if pos.y() < origin.y() {
            if inside_rim { bedrock } else { end_stone }
        } else if pos.y() > origin.y() {
            air
        } else if !inside_rim {
            bedrock
        } else if active {
            end_portal
        } else {
            air
        };
        set_podium_block(world, pos, state);
    }

    // Four-block bedrock pillar in the centre, with one torch on each side.
    for dy in 0..4 {
        set_podium_block(world, origin.above_n(dy), bedrock);
    }
    let center = origin.above_n(2);
    for facing in [
        Direction::North,
        Direction::South,
        Direction::West,
        Direction::East,
    ] {
        let torch = vanilla_blocks::WALL_TORCH
            .default_state()
            .set_value(&BlockStateProperties::HORIZONTAL_FACING, facing);
        set_podium_block(world, center.relative(facing), torch);
    }
}

fn set_podium_block(world: &Arc<World>, pos: BlockPos, state: steel_utils::BlockStateId) {
    // `set_block` reports an unchanged position as failure; the podium is
    // idempotent, so every result is intentionally ignored.
    let _ = world.set_block(pos, state, UpdateFlags::UPDATE_ALL);
}

/// The 9x9 podium footprint touches chunks (-1,-1) through (0,0).
fn end_podium_chunks_loaded(world: &Arc<World>) -> bool {
    [
        ChunkPos::new(-1, -1),
        ChunkPos::new(-1, 0),
        ChunkPos::new(0, -1),
        ChunkPos::new(0, 0),
    ]
    .into_iter()
    .all(|pos| world.chunk_map.is_block_ticking_full_chunk_loaded(pos))
}

fn remove_legacy_end_gateways(world: &Arc<World>, podium: BlockPos, y: i32) {
    let centers = [(6, 0), (-6, 0), (0, 6), (0, -6)];
    let all_legacy = centers.iter().all(|(dx, dz)| {
        world.get_block_state(BlockPos::new(podium.x() + *dx, y, podium.z() + *dz)).get_block()
            == &vanilla_blocks::END_GATEWAY
    });
    if !all_legacy {
        return;
    }

    // `create_end_gateway_portal` builds a 3x3x5 bedrock cage around each
    // gateway. Remove exactly that pattern while leaving mismatched blocks
    // alone, so a player-modified structure is not damaged further.
    for (cx, cz) in centers {
        let origin = BlockPos::new(podium.x() + cx, y, podium.z() + cz);
        for dy in -2_i32..=2 {
            for dx in -1..=1 {
                for dz in -1..=1 {
                    let center = dx == 0 && dy == 0 && dz == 0;
                    let end = dy.abs() == 2;
                    let frame = (end && dx == 0 && dz == 0)
                        || ((dx == 0 || dz == 0) && !end);
                    if !center && !frame {
                        continue;
                    }
                    let expected = if center {
                        &vanilla_blocks::END_GATEWAY
                    } else {
                        &vanilla_blocks::BEDROCK
                    };
                    let pos = origin.offset(dx, dy, dz);
                    if world.get_block_state(pos).get_block() == expected {
                        let _ = world.set_block(
                            pos,
                            vanilla_blocks::AIR.default_state(),
                            UpdateFlags::UPDATE_ALL,
                        );
                    }
                }
            }
        }
    }
}

fn find_dragon_egg(world: &Arc<World>, podium: BlockPos, y: i32) -> Option<BlockPos> {
    (0..=5)
        .map(|dy| BlockPos::new(podium.x(), y + dy, podium.z()))
        .find(|pos| world.get_block_state(*pos).get_block() == &vanilla_blocks::DRAGON_EGG)
}

/// Vanilla `EndDragonFight` ring: `floor(96 * cos(PI*i/10))`,
/// `floor(96 * sin(PI*i/10))`, always at Y=75.
fn end_gateway_position(index: i32) -> BlockPos {
    let angle = std::f64::consts::PI * f64::from(index) / 10.0;
    BlockPos::new(
        (END_GATEWAY_RING_RADIUS * angle.cos()).floor() as i32,
        END_GATEWAY_Y,
        (END_GATEWAY_RING_RADIUS * angle.sin()).floor() as i32,
    )
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

    use super::{END_GATEWAY_Y, EndFightState, PersistedEndFight};
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
        // The podium keeps a bedrock pillar on the centre column, so the portal
        // ring sits one block east of the centre.
        let portal = (60..80)
            .find(|y| {
                world
                    .get_block_state(BlockPos::new(podium.x() + 1, *y, podium.z()))
                    .get_block()
                    == vanilla_blocks::END_PORTAL.default_state().get_block()
            })
            .expect("return portal should exist at the podium");
        assert_eq!(
            world.get_block_state(BlockPos::new(podium.x(), portal + 4, podium.z())).get_block(),
            vanilla_blocks::DRAGON_EGG.default_state().get_block()
        );
        // The rim is bedrock at the portal layer; the old 5x5 bedrock base
        // below the portal was the placeholder this fix removes.
        assert_eq!(
            world
                .get_block_state(BlockPos::new(podium.x() + 2, portal, podium.z() + 2))
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
    fn killing_the_dragon_spawns_one_gateway_on_the_outer_ring() {
        let world = end_world("gateways");
        let mut fight = EndFightState::default();
        fight.tick(&world);
        assert!(!fight.is_killed());
        // The inactive podium already exists before the kill; only its portal
        // blocks are a kill reward.
        assert!(fight.podium_placed);

        world.on_ender_dragon_killed();
        assert!(world.is_ender_dragon_killed());

        // Vanilla opens one gateway per dragon kill, on the 96-block ring at
        // Y=75, picked by world seed. The world's own fight state owns the
        // delayed queue, not the local shadow used above.
        let mut random = steel_utils::random::legacy_random::LegacyRandom::from_seed(
            world.seed() as u64,
        );
        let index = steel_utils::random::Random::next_i32_bounded(
            &mut random,
            super::END_GATEWAY_SLOTS,
        );
        let gateway_pos = super::end_gateway_position(index);
        assert_eq!(gateway_pos.y(), END_GATEWAY_Y);
        let distance = f64::from(gateway_pos.x()).hypot(f64::from(gateway_pos.z()));
        assert!(distance > 90.0, "gateway must sit on the outer ring, got {gateway_pos:?}");

        let is_delayed = {
            let state = world.end_fight.lock();
            assert!(state.pending_gateways.len() <= 1);
            state.pending_gateways.contains(&gateway_pos)
        };
        if is_delayed {
            // Load the chunks touching the ring position; the delayed gateway
            // then materializes exactly like `END_GATEWAY_DELAYED`.
            let chunk = steel_utils::ChunkPos::new(gateway_pos.x() >> 4, gateway_pos.z() >> 4);
            for cx in (chunk.0.x - 1)..=(chunk.0.x + 1) {
                for cz in (chunk.0.y - 1)..=(chunk.0.y + 1) {
                    insert_ready_full_chunk(&world, steel_utils::ChunkPos::new(cx, cz));
                }
            }
            world.end_fight.lock().tick(&world);
            assert!(world.end_fight.lock().pending_gateways.is_empty());
        }

        assert_eq!(
            world.get_block_state(gateway_pos).get_block(),
            vanilla_blocks::END_GATEWAY.default_state().get_block()
        );
        let block_entity = world
            .get_block_entity(gateway_pos)
            .expect("delayed gateway must have a block entity");
        let gateway = block_entity
            .downcast_ref::<crate::block_entity::entities::EndGatewayBlockEntity>()
            .expect("block entity should be an end gateway");
        assert_eq!(
            gateway.exit_portal(),
            None,
            "post-kill gateway exit must be unassigned, not a placeholder coordinate"
        );
        assert!(!gateway.exact_teleport());
        assert!(gateway.is_spawning());
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
                    .get_block_state(BlockPos::new(podium.x() + 1, *y, podium.z()))
                    .get_block()
                    == vanilla_blocks::END_PORTAL.default_state().get_block()
            })
            .expect("real dragon death must build the return portal");
        assert!(
            world
                .get_block_state(BlockPos::new(podium.x(), portal + 4, podium.z()))
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
