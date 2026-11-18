use steel_registry::blocks::properties::{BlockStateProperties, Direction};
use steel_registry::init_vanilla_registry;
use steel_utils::{ChunkPos, Downcast as _, WorldAabb};

use super::*;
use crate::behavior::init_behaviors;
use crate::entity::SharedEntity;
use crate::player::ResetReason;
use crate::test_support::{TestPlayerBuilder, fresh_test_world, insert_ready_full_chunk};
use crate::world::{ExplosionInteraction, ExplosionOptions};

#[test]
fn adventure_owner_primes_only_when_the_igniting_stack_can_break_tnt() {
    init_vanilla_registry();
    init_behaviors();
    let fixture = fresh_test_world("tnt_adventure_priming");
    let world = &fixture.world;
    let pos = BlockPos::new(8, 64, 8);
    insert_ready_full_chunk(world, ChunkPos::from_block_pos(pos));
    let owner =
        TestPlayerBuilder::new(Arc::clone(world), "AdventureOwner", next_entity_id()).build();
    owner.restore_game_modes(GameType::Adventure, None);

    assert!(!TntBlock::prime_with_source(
        world,
        pos,
        Some(&*owner),
        || false
    ));
    assert!(TntBlock::prime_with_source(
        world,
        pos,
        Some(&*owner),
        || true
    ));
}

#[test]
fn explosion_replaces_tnt_with_a_short_fuse_entity_owned_by_the_igniter() {
    init_vanilla_registry();
    init_behaviors();
    let fixture = fresh_test_world("tnt_chain_reaction");
    let world = &fixture.world;
    let pos = BlockPos::new(8, 64, 8);
    insert_ready_full_chunk(world, ChunkPos::from_block_pos(pos));
    assert!(world.set_block(
        pos,
        vanilla_blocks::TNT.default_state(),
        UpdateFlags::UPDATE_NONE,
    ));
    let owner = TestPlayerBuilder::new(Arc::clone(world), "ChainOwner", next_entity_id()).build();
    owner
        .base()
        .set_position_local(pos.offset(4, 0, 0).get_bottom_center());
    assert!(world.add_player(Arc::clone(&owner), ResetReason::InitialJoin));
    let mut options = ExplosionOptions::new(pos.get_center(), 4.0, ExplosionInteraction::Tnt);
    options.source = Some(Arc::clone(&owner) as SharedEntity);

    world.explode(options);

    assert!(world.get_block_state(pos).is_air());
    let (x, y, z) = (f64::from(pos.x()), f64::from(pos.y()), f64::from(pos.z()));
    let primed = world
        .get_entities_in_aabb(&WorldAabb::new(x, y, z, x + 1.0, y + 1.0, z + 1.0))
        .into_iter()
        .filter(|entity| entity.downcast_ref::<PrimedTntEntity>().is_some())
        .collect::<Vec<_>>();
    let [primed] = primed.as_slice() else {
        panic!("expected exactly one primed TNT, found {}", primed.len());
    };
    let fuse = primed
        .downcast_ref::<PrimedTntEntity>()
        .map(PrimedTntEntity::fuse)
        .expect("spawned entity should remain primed TNT");
    // `getRandomShortFuse(80)` is `nextInt(20) + 10`.
    assert!((10..30).contains(&fuse), "fuse={fuse}");
    assert_eq!(
        primed.explosion_indirect_source().map(|source| source.id()),
        Some(owner.id())
    );
}

fn tick_moving_block(world: &Arc<World>, pos: BlockPos, ticks: usize) {
    for _ in 0..ticks {
        let Some(block_entity) = world.get_block_entity(pos) else {
            break;
        };
        block_entity.tick(world);
    }
}

#[test]
fn sticky_piston_tnt_duper_keeps_the_tnt_block_and_primes_each_pass() {
    init_vanilla_registry();
    init_behaviors();
    let fixture = fresh_test_world("tnt_duper_cycle");
    let world = &fixture.world;
    let piston_pos = BlockPos::new(8, 64, 8);
    let tnt_pos = BlockPos::new(9, 64, 8);
    let destination = BlockPos::new(10, 64, 8);
    let power_pos = BlockPos::new(7, 64, 8);
    let destination_power = BlockPos::new(10, 64, 9);
    insert_ready_full_chunk(world, ChunkPos::from_block_pos(piston_pos));

    let piston_state = vanilla_blocks::STICKY_PISTON
        .default_state()
        .set_value(&BlockStateProperties::FACING, Direction::East)
        .set_value(&BlockStateProperties::EXTENDED, false);
    assert!(world.set_block(piston_pos, piston_state, UpdateFlags::UPDATE_NONE));
    assert!(world.set_block(
        tnt_pos,
        vanilla_blocks::TNT.default_state(),
        UpdateFlags::UPDATE_NONE
    ));
    assert!(world.set_block(
        destination_power,
        vanilla_blocks::REDSTONE_BLOCK.default_state(),
        UpdateFlags::UPDATE_NONE
    ));
    assert!(world.set_block(
        power_pos,
        vanilla_blocks::REDSTONE_BLOCK.default_state(),
        UpdateFlags::UPDATE_ALL
    ));
    world.run_block_events();
    // Finalize the moved TNT first, then remove the pulse before the piston
    // head settles. This mirrors the one-tick power pulse of a real duper.
    tick_moving_block(world, destination, 3);
    assert!(world.remove_block(destination_power, false));
    tick_moving_block(world, tnt_pos, 3);

    assert_eq!(
        world.get_block_state(destination).get_block(),
        &vanilla_blocks::TNT,
        "the moved TNT block must survive the duper update"
    );
    let count_tnt = |world: &Arc<World>| {
        world
            .get_entities_in_aabb(&WorldAabb::new(9.0, 63.0, 7.0, 11.0, 65.0, 9.0))
            .into_iter()
            .filter(|entity| entity.entity_type() == &vanilla_entities::TNT)
            .count()
    };
    assert_eq!(count_tnt(world), 1, "first pass must prime one TNT entity");

    assert!(world.remove_block(power_pos, false));
    world.run_block_events();
    tick_moving_block(world, piston_pos, 3);
    tick_moving_block(world, tnt_pos, 3);
    assert_eq!(
        world.get_block_state(tnt_pos).get_block(),
        &vanilla_blocks::TNT,
        "sticky piston must pull the TNT block back"
    );

    assert!(world.set_block(
        destination_power,
        vanilla_blocks::REDSTONE_BLOCK.default_state(),
        UpdateFlags::UPDATE_NONE
    ));
    assert!(world.set_block(
        power_pos,
        vanilla_blocks::REDSTONE_BLOCK.default_state(),
        UpdateFlags::UPDATE_ALL
    ));
    world.run_block_events();
    tick_moving_block(world, destination, 3);
    assert!(world.remove_block(destination_power, false));
    tick_moving_block(world, tnt_pos, 3);

    assert_eq!(
        world.get_block_state(destination).get_block(),
        &vanilla_blocks::TNT,
        "second pass must keep the same TNT block"
    );
    assert_eq!(count_tnt(world), 2, "second pass must prime a second TNT entity");
}
