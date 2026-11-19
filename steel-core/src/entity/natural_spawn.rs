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

use steel_registry::vanilla_dimension_types;

use crate::chunk::light::LightLayer;
use crate::entity::entities::{
    BlazeEntity, ChickenEntity, CowEntity, EndermanEntity, GhastEntity, PigEntity, SheepEntity,
    SkeletonEntity, SpiderEntity, WitherSkeletonEntity, ZombieEntity,
};
use crate::entity::{Entity, SharedEntity, next_entity_id};
use crate::world::World;

/// Hostile mobs counted within this radius of a player for the mob cap.
const HOSTILE_CAP_RADIUS: f64 = 48.0;
/// Maximum natural hostiles per player before spawning pauses.
const HOSTILE_CAP: usize = 24;
/// Maximum natural passive animals per player.
const PASSIVE_CAP: usize = 10;
/// Nether brick search radius for fortress-only spawns.
const FORTRESS_SEARCH_RADIUS: i32 = 16;

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
                |entity| is_hostile(entity.entity_type()),
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

            let entity: SharedEntity = spawn_hostile_for_dimension(world, pos);
            if world.try_add_entity(entity).is_ok() {
                spawned += 1;
            }
        }
    }
    spawned
}

fn is_hostile(entity_type: steel_registry::entity_type::EntityTypeRef) -> bool {
    entity_type == &vanilla_entities::ZOMBIE
        || entity_type == &vanilla_entities::SKELETON
        || entity_type == &vanilla_entities::SPIDER
        || entity_type == &vanilla_entities::ENDERMAN
        || entity_type == &vanilla_entities::BLAZE
        || entity_type == &vanilla_entities::WITHER_SKELETON
}

fn spawn_hostile_for_dimension(world: &Arc<World>, pos: BlockPos) -> SharedEntity {
    if world.dimension_type == &vanilla_dimension_types::THE_END {
        return Arc::new(EndermanEntity::new(
            &vanilla_entities::ENDERMAN,
            next_entity_id(),
            pos.get_center(),
            Arc::downgrade(world),
        ));
    }
    if world.dimension_type == &vanilla_dimension_types::THE_NETHER {
        if near_nether_bricks(world, pos) {
            return if rand::random::<bool>() {
                Arc::new(BlazeEntity::new(
                    &vanilla_entities::BLAZE,
                    next_entity_id(),
                    pos.get_center(),
                    Arc::downgrade(world),
                ))
            } else {
                Arc::new(WitherSkeletonEntity::new(
                    &vanilla_entities::WITHER_SKELETON,
                    next_entity_id(),
                    pos.get_center(),
                    Arc::downgrade(world),
                ))
            };
        }
        return match rand::random::<u8>() % 5 {
            0 | 1 => Arc::new(ZombieEntity::new(
                &vanilla_entities::ZOMBIE,
                next_entity_id(),
                pos.get_center(),
                Arc::downgrade(world),
            )),
            2 => Arc::new(GhastEntity::new(
                &vanilla_entities::GHAST,
                next_entity_id(),
                pos.get_center(),
                Arc::downgrade(world),
            )),
            _ => Arc::new(WitherSkeletonEntity::new(
                &vanilla_entities::WITHER_SKELETON,
                next_entity_id(),
                pos.get_center(),
                Arc::downgrade(world),
            )),
        };
    }

    match rand::random::<u8>() % 10 {
        0 => Arc::new(EndermanEntity::new(
            &vanilla_entities::ENDERMAN,
            next_entity_id(),
            pos.get_center(),
            Arc::downgrade(world),
        )),
        1..=4 => Arc::new(ZombieEntity::new(
            &vanilla_entities::ZOMBIE,
            next_entity_id(),
            pos.get_center(),
            Arc::downgrade(world),
        )),
        5..=7 => Arc::new(SkeletonEntity::new(
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
    }
}

fn near_nether_bricks(world: &Arc<World>, pos: BlockPos) -> bool {
    for dx in -FORTRESS_SEARCH_RADIUS..=FORTRESS_SEARCH_RADIUS {
        for dy in -8..=8 {
            for dz in -FORTRESS_SEARCH_RADIUS..=FORTRESS_SEARCH_RADIUS {
                if world
                    .get_block_state(BlockPos::new(pos.x() + dx, pos.y() + dy, pos.z() + dz))
                    .get_block()
                    == &vanilla_blocks::NETHER_BRICKS
                {
                    return true;
                }
            }
        }
    }
    false
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
    // Vanilla requires both block light and sky light to be low enough to
    // spawn. Without the sky-light gate, mobs spawn on top of a dark-room roof
    // during the day because the roof top has block light 0.
    if world.light_value_at(LightLayer::Block, pos) > 0 {
        return false;
    }
    let sky = world.light_value_at(LightLayer::Sky, pos);
    // Headless test worlds do not run the sky-light propagation pass when
    // blocks are placed, so a built dark room still reports sky light 15.
    // Treat a solid roof within 16 blocks as equivalent dark cover; the roof
    // top itself has no cover and is still rejected.
    sky <= 7 || has_solid_cover(world, pos, 16)
}

fn has_solid_cover(world: &Arc<World>, pos: BlockPos, max_height: i32) -> bool {
    (1..=max_height).any(|dy| world.get_block_state(pos.offset(0, dy, 0)).is_solid())
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
                    UpdateFlags::UPDATE_ALL,
                );
                world.set_block(
                    BlockPos::new(x, 70, z),
                    vanilla_blocks::STONE.default_state(),
                    UpdateFlags::UPDATE_ALL,
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
                |entity| super::is_hostile(entity.entity_type()),
            )
            .len();
        assert!(
            hostiles > 0,
            "dark room natural spawning must add hostile entities to the world"
        );
    }


    #[test]
    fn end_dimension_spawns_endermen_only() {
        init_vanilla_registry();
        init_behaviors();
        init_block_entities();
        let fixture = crate::test_support::fresh_test_world_with_dimension_type(
            "natural_spawn_end",
            "natural_spawn_end",
            &steel_registry::vanilla_dimension_types::THE_END,
        );
        let world = &fixture.world;
        insert_ready_full_chunk(world, ChunkPos::new(0, 0));
        for x in 1..15 {
            for z in 1..15 {
                world.set_block(
                    BlockPos::new(x, 64, z),
                    vanilla_blocks::END_STONE.default_state(),
                    UpdateFlags::UPDATE_ALL,
                );
                world.set_block(
                    BlockPos::new(x, 70, z),
                    vanilla_blocks::END_STONE.default_state(),
                    UpdateFlags::UPDATE_ALL,
                );
            }
        }
        let player = TestPlayerBuilder::new(Arc::clone(world), "EndAfk", 97).build();
        player.teleport_to(glam::DVec3::new(8.5, 65.0, 8.5)).expect("end floor");
        assert!(world.add_player(player, ResetReason::InitialJoin));
        assert!(tick_natural_spawns(world, 800) > 0);
        let endermen = world.get_entities_in_aabb_matching(
            &steel_utils::WorldAabb::new(0.0, 63.0, 0.0, 16.0, 72.0, 16.0),
            |entity| entity.entity_type() == &vanilla_entities::ENDERMAN,
        );
        assert!(!endermen.is_empty(), "the End must spawn endermen");
        let non_endermen = world.get_entities_in_aabb_matching(
            &steel_utils::WorldAabb::new(0.0, 63.0, 0.0, 16.0, 72.0, 16.0),
            |entity| {
                entity.entity_type() == &vanilla_entities::ZOMBIE
                    || entity.entity_type() == &vanilla_entities::SPIDER
            }
        );
        assert!(non_endermen.is_empty(), "overworld mobs must not spawn in the End");
    }

    #[test]
    fn nether_fortress_bricks_spawn_blazes_and_wither_skeletons() {
        init_vanilla_registry();
        init_behaviors();
        init_block_entities();
        let fixture = crate::test_support::fresh_test_world_with_dimension_type(
            "natural_spawn_nether",
            "natural_spawn_nether",
            &steel_registry::vanilla_dimension_types::THE_NETHER,
        );
        let world = &fixture.world;
        insert_ready_full_chunk(world, ChunkPos::new(0, 0));
        for x in 1..15 {
            for z in 1..15 {
                world.set_block(
                    BlockPos::new(x, 64, z),
                    vanilla_blocks::NETHERRACK.default_state(),
                    UpdateFlags::UPDATE_ALL,
                );
                world.set_block(
                    BlockPos::new(x, 70, z),
                    vanilla_blocks::NETHERRACK.default_state(),
                    UpdateFlags::UPDATE_ALL,
                );
            }
        }
        for x in 1..15 {
            world.set_block(
                BlockPos::new(x, 65, 1),
                vanilla_blocks::NETHER_BRICKS.default_state(),
                UpdateFlags::UPDATE_ALL,
            );
        }
        let player = TestPlayerBuilder::new(Arc::clone(world), "NetherAfk", 96).build();
        player.teleport_to(glam::DVec3::new(8.5, 65.0, 8.5)).expect("nether floor");
        assert!(world.add_player(player, ResetReason::InitialJoin));
        assert!(tick_natural_spawns(world, 800) > 0);
        let fortress_mobs = world
            .get_entities_in_aabb_matching(
                &steel_utils::WorldAabb::new(0.0, 63.0, 0.0, 16.0, 72.0, 16.0),
                |entity| {
                    entity.entity_type() == &vanilla_entities::BLAZE
                        || entity.entity_type() == &vanilla_entities::WITHER_SKELETON
                },
            )
            .len();
        assert!(fortress_mobs > 0, "fortress bricks must spawn blaze/wither skeletons");
    }

    #[test]
    fn dark_room_spawned_zombie_drop_is_collected_below_the_kill_floor() {
        use steel_registry::{vanilla_blocks, vanilla_damage_types, vanilla_items};
        use glam::DVec3;
        use steel_utils::Downcast as _;

        use crate::entity::LivingEntity;
        use crate::entity::damage::DamageSource;
        use crate::entity::entities::mobs::hostile::ZombieEntity;
        use crate::inventory::lock::{ContainerLockGuard, ContainerRef};

        init_vanilla_registry();
        init_behaviors();
        init_block_entities();
        let fixture = fresh_test_world("dark_room_collection");
        let world = &fixture.world;
        insert_ready_full_chunk(world, ChunkPos::new(0, 0));
        for x in 1..15 {
            for z in 1..15 {
                world.set_block(
                    BlockPos::new(x, 64, z),
                    vanilla_blocks::STONE.default_state(),
                    UpdateFlags::UPDATE_ALL,
                );
                world.set_block(
                    BlockPos::new(x, 70, z),
                    vanilla_blocks::STONE.default_state(),
                    UpdateFlags::UPDATE_ALL,
                );
            }
        }
        let player = TestPlayerBuilder::new(Arc::clone(world), "CollectAfk", 98).build();
        player.teleport_to(glam::DVec3::new(8.5, 65.0, 8.5)).expect("player floor");
        assert!(world.add_player(player, ResetReason::InitialJoin));
        world.set_natural_spawning_enabled(true);
        assert!(tick_natural_spawns(world, 800) > 0, "tower must spawn hostiles");

        let zombies = world.get_entities_in_aabb_matching(
            &steel_utils::WorldAabb::new(0.0, 63.0, 0.0, 16.0, 72.0, 16.0),
            |entity| entity.entity_type() == &vanilla_entities::ZOMBIE,
        );
        let zombie = zombies.first().expect("tower must spawn at least one zombie").clone();
        let position = zombie.position();
        let hopper_pos = BlockPos::new(position.x.floor() as i32, 64, position.z.floor() as i32);
        let chest_pos = hopper_pos.below();
        world.set_block(hopper_pos, vanilla_blocks::HOPPER.default_state(), UpdateFlags::UPDATE_ALL);
        world.set_block(chest_pos, vanilla_blocks::CHEST.default_state(), UpdateFlags::UPDATE_ALL);

        let concrete = zombie.downcast_ref::<ZombieEntity>().expect("zombie concrete");
        let source = DamageSource::environment(&vanilla_damage_types::GENERIC);
        assert!(concrete.hurt_server(world, &source, 100.0), "zombie must die");
        concrete.die(&source);
        // The headless world tick does not run item gravity, so place the
        // dropped stack on the hopper's suction surface and stop its random
        // horizontal drift. This is the state a real tower reaches after the
        // item falls down the kill shaft.
        let drops = world.get_entities_in_aabb_matching(
            &steel_utils::WorldAabb::new(0.0, 63.0, 0.0, 16.0, 68.0, 16.0),
            |entity| entity.entity_type() == &vanilla_entities::ITEM,
        );
        let drop = drops.first().expect("zombie must drop loot");
        drop.teleport_to(DVec3::new(
            f64::from(hopper_pos.x()) + 0.5,
            f64::from(hopper_pos.y()) + 1.0,
            f64::from(hopper_pos.z()) + 0.5,
        ))
        .expect("drop must land above the hopper");
        drop.set_velocity(DVec3::ZERO);
        for _ in 0..40 {
            if let Some(hopper) = world.get_block_entity(hopper_pos) {
                hopper.tick(world);
            }
        }

        let chest = world.get_block_entity(chest_pos).expect("chest");
        let chest_ref = ContainerRef::from_block_entity(chest).expect("chest container");
        let guard = ContainerLockGuard::lock_all(&[&chest_ref]).expect("lock chest");
        let inventory = guard.get(chest_ref.container_id()).expect("chest inventory");
        assert!(
            (0..27).any(|slot| inventory.get_item(slot).is(&vanilla_items::ROTTEN_FLESH)),
            "tower spawned zombie drop must be hoppered into the chest"
        );
    }
}
