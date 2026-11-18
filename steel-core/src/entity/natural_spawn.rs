//! Minimal vanilla-style natural hostile spawning.
//!
//! The server calls this from the world tick when natural spawning is enabled.
//! It picks dark, solid-supported air blocks near players and spawns the three
//! implemented hostile mobs (zombie, skeleton, spider), respecting a simple
//! per-player hostile cap. The dark-room projection test drives it directly.

use std::sync::Arc;

use steel_registry::blocks::block_state_ext::BlockStateExt as _;
use steel_registry::{vanilla_blocks, vanilla_entities};
use steel_utils::BlockPos;

use crate::chunk::light::LightLayer;
use crate::entity::entities::{SkeletonEntity, SpiderEntity, ZombieEntity};
use crate::entity::{Entity, SharedEntity, next_entity_id};
use crate::world::World;

/// Hostile mobs counted within this radius of a player for the mob cap.
const HOSTILE_CAP_RADIUS: f64 = 48.0;
/// Maximum natural hostiles per player before spawning pauses.
const HOSTILE_CAP: usize = 24;

/// Tries to spawn naturally near every player. Returns the number spawned.
pub fn tick_natural_spawns(world: &Arc<World>, attempts_per_player: usize) -> usize {
    let mut players = Vec::new();
    world.players.iter_players(|_, player| {
        players.push((player.position(), player.id()));
        false
    });
    let mut spawned = 0;
    for (player_pos, _) in players {
        let hostiles = world
            .get_entities_in_aabb_matching(
                &steel_utils::WorldAabb::new(
                    player_pos.x - HOSTILE_CAP_RADIUS,
                    player_pos.y - HOSTILE_CAP_RADIUS,
                    player_pos.z - HOSTILE_CAP_RADIUS,
                    player_pos.x + HOSTILE_CAP_RADIUS,
                    player_pos.y + HOSTILE_CAP_RADIUS,
                    player_pos.z + HOSTILE_CAP_RADIUS,
                ),
                |entity| {
                    entity.entity_type() == &vanilla_entities::ZOMBIE
                        || entity.entity_type() == &vanilla_entities::SKELETON
                        || entity.entity_type() == &vanilla_entities::SPIDER
                },
            )
            .len();
        if hostiles >= HOSTILE_CAP {
            continue;
        }

        for _ in 0..attempts_per_player {
            let x = player_pos.x + (rand::random::<f64>() - 0.5) * 32.0;
            let z = player_pos.z + (rand::random::<f64>() - 0.5) * 32.0;
            let y = player_pos.y + (rand::random::<f64>() - 0.5) * 16.0;
            let pos = BlockPos::new(x.floor() as i32, y.floor() as i32, z.floor() as i32);
            if !is_dark_spawnable(world, pos) {
                continue;
            }

            let entity: SharedEntity = match rand::random::<u8>() % 3 {
                0 => Arc::new(ZombieEntity::new(
                    &vanilla_entities::ZOMBIE,
                    next_entity_id(),
                    pos.get_center(),
                    Arc::downgrade(world),
                )),
                1 => Arc::new(SkeletonEntity::new(
                    &vanilla_entities::SKELETON,
                    next_entity_id(),
                    pos.get_center(),
                    Arc::downgrade(world),
                )),
                _ => Arc::new(SpiderEntity::new(
                    &vanilla_entities::SPIDER,
                    next_entity_id(),
                    pos.get_center(),
                    Arc::downgrade(world),
                )),
            };
            if world.try_add_entity(entity).is_ok() {
                spawned += 1;
            }
        }
    }
    spawned
}

fn is_dark_spawnable(world: &Arc<World>, pos: BlockPos) -> bool {
    if !world.is_in_valid_bounds(pos) {
        return false;
    }
    if world.get_block_state(pos).get_block() != &vanilla_blocks::AIR {
        return false;
    }
    let below = world.get_block_state(pos.below());
    if below.get_block() == &vanilla_blocks::AIR || below.get_block() == &vanilla_blocks::WATER {
        return false;
    }
    // Dark-room spawning: block light must be zero. Sky light is still being
    // wired into chunk light updates, so it is not used as a spawn gate here.
    world.light_value_at(LightLayer::Block, pos) == 0
}

/// Vanilla-like spawn position helper exposed for machine tests.
#[must_use]
pub fn spawn_position_is_valid(world: &Arc<World>, pos: BlockPos) -> bool {
    is_dark_spawnable(world, pos)
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use steel_registry::{init_vanilla_registry, vanilla_blocks, vanilla_entities};
    use steel_utils::{BlockPos, ChunkPos, types::UpdateFlags};

    use crate::behavior::init_behaviors;
    use crate::block_entity::init_block_entities;
    use crate::entity::Entity;
    use crate::entity::natural_spawn::tick_natural_spawns;
    use crate::player::ResetReason;
    use crate::test_support::{
        TestPlayerBuilder, fresh_test_world, insert_ready_full_chunk, tick_test_world,
    };

    #[test]
    fn dark_room_natural_spawning_spawns_hostiles() {
        init_vanilla_registry();
        init_behaviors();
        init_block_entities();
        let fixture = fresh_test_world("natural_spawn_dark_room");
        let world = &fixture.world;
        insert_ready_full_chunk(world, ChunkPos::new(0, 0));

        // 14x14 stone spawn floor at y=64 with a roof at y=70.
        for x in 1..15 {
            for z in 1..15 {
                world.set_block(
                    BlockPos::new(x, 64, z),
                    vanilla_blocks::STONE.default_state(),
                    UpdateFlags::UPDATE_CLIENTS,
                );
                world.set_block(
                    BlockPos::new(x, 70, z),
                    vanilla_blocks::STONE.default_state(),
                    UpdateFlags::UPDATE_CLIENTS,
                );
            }
        }

        let player = TestPlayerBuilder::new(Arc::clone(world), "SpawnAfk", 99).build();
        player
            .teleport_to(glam::DVec3::new(8.5, 65.0, 8.5))
            .expect("player should stand on the dark room floor");
        assert!(world.add_player(player, ResetReason::InitialJoin));

        assert_eq!(world.players.len(), 1, "player must be registered for spawning");
        assert!(
            crate::entity::natural_spawn::spawn_position_is_valid(world, BlockPos::new(8, 65, 8)),
            "dark room air block above the stone floor must be spawnable"
        );
        world.set_natural_spawning_enabled(true);
        tick_test_world(world, 200, true);
        // The world tick only tries a few spawns per 20 ticks; drive a larger
        // sample directly so the test is not flaky at low light-scan density.
        let direct = tick_natural_spawns(world, 800);
        assert!(direct > 0, "dark room must produce natural hostiles");

        let hostiles = world
            .get_entities_in_aabb_matching(
                &steel_utils::WorldAabb::new(0.0, 63.0, 0.0, 16.0, 72.0, 16.0),
                |entity| {
                    entity.entity_type() == &vanilla_entities::ZOMBIE
                        || entity.entity_type() == &vanilla_entities::SKELETON
                        || entity.entity_type() == &vanilla_entities::SPIDER
                },
            )
            .len();
        assert!(
            hostiles > 0,
            "dark room natural spawning must add hostile entities to the world"
        );
    }
}
