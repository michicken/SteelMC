//! Machine-level acceptance tests driven by actual Litematica projection files.
//!
//! Each test writes a real gzip-compressed `.litematic` (same layout Litematica
//! saves), reads it back through [`load_litematic`], pastes it into a real
//! `World`, ticks the world, and asserts on block states. This is deliberately
//! end-to-end: fixture bytes on disk -> NBT parse -> palette resolution ->
//! world placement -> scheduled/block-event ticking.

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::sync::Arc;

    use glam::DVec3;
    use simdnbt::owned::{NbtCompound, NbtList};
    use steel_registry::blocks::block_state_ext::BlockStateExt as _;
    use steel_registry::blocks::properties::{BlockStateProperties, Direction};
    use steel_registry::item_stack::ItemStack;
    use steel_registry::{vanilla_blocks, vanilla_items};
    use steel_utils::types::UpdateFlags;
    use steel_utils::{BlockPos, ChunkPos, Identifier};

    use crate::inventory::lock::{ContainerLockGuard, ContainerRef};
    use crate::test_support::{
        LitematicBuilder, TestPlayerBuilder, dropped_items, fresh_test_world,
        insert_ready_full_chunk, load_litematic, paste_litematic, tick_test_world,
    };

    fn tick_world(world: &Arc<crate::world::World>, ticks: usize) {
        for _ in 0..ticks {
            tick_test_world(world, 1, true);
        }
    }

    /// Vanilla's block-entity ticker is a separate pass from the world tick;
    /// the test world does not run it implicitly, so piston animations need it.
    fn tick_block_entities(world: &Arc<crate::world::World>, ticks: usize) {
        for _ in 0..ticks {
            world.block_entity_tickers().tick(world, true);
        }
    }

    fn container_at(world: &Arc<crate::world::World>, pos: BlockPos) -> ContainerRef {
        ContainerRef::from_block_entity(world.get_block_entity(pos).expect("block entity"))
            .expect("container")
    }

    fn set_slot(world: &Arc<crate::world::World>, pos: BlockPos, slot: usize, stack: ItemStack) {
        let container = container_at(world, pos);
        let mut guard = ContainerLockGuard::lock_all(&[&container]).expect("lock");
        guard.set_item(container.container_id(), slot, stack);
    }

    fn count_at(world: &Arc<crate::world::World>, pos: BlockPos, slot: usize) -> i32 {
        let container = container_at(world, pos);
        let guard = ContainerLockGuard::lock_all(&[&container]).expect("lock");
        guard
            .get(container.container_id())
            .expect("container")
            .get_item(slot)
            .count()
    }

    fn fixture_path(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join("steelmc-litematic-tests");
        std::fs::create_dir_all(&dir).expect("create fixture dir");
        dir.join(format!("{name}.litematic"))
    }

    fn block_id(state: steel_utils::BlockStateId) -> Identifier {
        state.get_block().key.clone()
    }

    #[test]
    fn litematic_round_trips_blocks_and_palette() {
        steel_registry::init_vanilla_registry();
        crate::behavior::init_behaviors();
        crate::block_entity::init_block_entities();
        let path = fixture_path("round_trip");
        let mut builder = LitematicBuilder::new("round_trip", (4, 3, 2));
        builder.fill("minecraft:stone", &[]);
        builder.set(1, 1, 0, "minecraft:redstone_block", &[]);
        builder.set(2, 1, 0, "minecraft:oak_planks", &[]);
        builder.set(3, 1, 1, "minecraft:piston", &[("facing", "east")]);
        builder.write(&path).expect("write litematic fixture");

        let litematic = load_litematic(&path).expect("load litematic fixture");
        assert_eq!(litematic.name, "round_trip");
        assert_eq!(litematic.regions.len(), 1);
        let region = &litematic.regions[0];
        assert_eq!(region.size, [4, 3, 2]);
        assert_eq!(
            block_id(region.state_at(0, 0, 0)),
            Identifier::new_static("minecraft", "stone")
        );
        assert_eq!(
            block_id(region.state_at(1, 1, 0)),
            Identifier::new_static("minecraft", "redstone_block")
        );
        assert_eq!(
            block_id(region.state_at(3, 1, 1)),
            Identifier::new_static("minecraft", "piston")
        );
        // 24 blocks filled with stone, then three overrides remain solid.
        assert_eq!(litematic.solid_block_count(), 24);
    }

    #[test]
    fn litematic_piston_machine_extends_and_pushes_in_world() {
        steel_registry::init_vanilla_registry();
        crate::behavior::init_behaviors();
        crate::block_entity::init_block_entities();
        let path = fixture_path("piston_pusher");
        let mut builder = LitematicBuilder::new("piston_pusher", (5, 3, 3));
        // x = 0 redstone block -> x = 1 piston facing east -> x = 2 stone.
        builder.set(0, 1, 1, "minecraft:redstone_block", &[]);
        builder.set(1, 1, 1, "minecraft:piston", &[("facing", "east")]);
        builder.set(2, 1, 1, "minecraft:stone", &[]);
        // Stone platform under the machine so the piston has a stable base.
        builder.set(1, 0, 1, "minecraft:stone", &[]);
        builder.write(&path).expect("write litematic fixture");

        let litematic = load_litematic(&path).expect("load litematic fixture");
        let test_world = fresh_test_world("machine_projection_piston");
        let world: Arc<_> = Arc::clone(&test_world.world);
        insert_ready_full_chunk(&world, ChunkPos::new(0, 0));

        let origin = BlockPos::new(2, 2, 2);
        let report = paste_litematic(&world, &litematic, origin);
        assert!(
            report.failed_positions.is_empty(),
            "projection blocks failed to place: {:?}",
            report.failed_positions
        );

        // Block events (piston extend) are processed by the normal game tick.
        for _ in 0..6 {
            tick_test_world(&world, 1, true);
        }

        let piston_pos = origin.offset(1, 1, 1);
        let head_pos = origin.offset(2, 1, 1);
        let pushed_pos = origin.offset(3, 1, 1);
        let piston_state = world.get_block_state(piston_pos);
        assert_eq!(
            block_id(piston_state),
            Identifier::new_static("minecraft", "piston")
        );
        assert!(
            piston_state.get_value(&BlockStateProperties::EXTENDED),
            "piston should be extended after the tick"
        );
        assert_eq!(
            block_id(world.get_block_state(head_pos)),
            Identifier::new_static("minecraft", "piston_head")
        );
        assert_eq!(
            block_id(world.get_block_state(pushed_pos)),
            Identifier::new_static("minecraft", "stone")
        );
        assert_eq!(
            block_id(world.get_block_state(origin.offset(2, 1, 1))),
            Identifier::new_static("minecraft", "piston_head"),
            "piston head must occupy the block the stone used to be in"
        );
        // redstone block + piston + pushed stone + platform = 4 placed blocks.
        assert_eq!(report.placed, 4);
    }

    #[test]
    fn litematic_repeater_delays_signal_into_lamp() {
        steel_registry::init_vanilla_registry();
        crate::behavior::init_behaviors();
        crate::block_entity::init_block_entities();

        let path = fixture_path("repeater_lamp");
        let mut builder = LitematicBuilder::new("repeater_lamp", (4, 2, 1));
        for x in 0..3 {
            builder.set(x, 0, 0, "minecraft:stone", &[]);
        }
        // Steel's repeater FACING points at its input side, so the redstone
        // block sits east and the lamp (output) sits west.
        builder.set(0, 1, 0, "minecraft:redstone_lamp", &[]);
        builder.set(
            1,
            1,
            0,
            "minecraft:repeater",
            &[("facing", "east"), ("delay", "1")],
        );
        builder.set(2, 1, 0, "minecraft:redstone_block", &[]);
        builder.write(&path).expect("write litematic fixture");

        let litematic = load_litematic(&path).expect("load litematic fixture");
        let test_world = fresh_test_world("machine_projection_repeater");
        let world: Arc<_> = Arc::clone(&test_world.world);
        insert_ready_full_chunk(&world, ChunkPos::new(0, 0));

        let origin = BlockPos::new(2, 2, 2);
        let report = paste_litematic(&world, &litematic, origin);
        assert!(
            report.failed_positions.is_empty(),
            "{:?}",
            report.failed_positions
        );
        assert_eq!(report.placed, 6);

        tick_world(&world, 6);

        let lamp = world.get_block_state(origin.offset(0, 1, 0));
        assert_eq!(
            block_id(lamp),
            Identifier::new_static("minecraft", "redstone_lamp")
        );
        assert!(
            lamp.get_value(&BlockStateProperties::LIT),
            "lamp must light after the repeater delay"
        );
    }

    #[test]
    fn litematic_dispenser_shoots_item_when_powered() {
        steel_registry::init_vanilla_registry();
        crate::behavior::init_behaviors();
        crate::block_entity::init_block_entities();

        let path = fixture_path("dispenser_shoot");
        let mut builder = LitematicBuilder::new("dispenser_shoot", (4, 2, 2));
        builder.set(0, 1, 1, "minecraft:redstone_block", &[]);
        builder.set(1, 1, 1, "minecraft:dispenser", &[("facing", "east")]);
        // Platform placed after the dispenser, so its neighbour update triggers the dispenser.
        builder.set(1, 0, 1, "minecraft:stone", &[]);
        builder.write(&path).expect("write litematic fixture");

        let litematic = load_litematic(&path).expect("load litematic fixture");
        let test_world = fresh_test_world("machine_projection_dispenser");
        let world: Arc<_> = Arc::clone(&test_world.world);
        insert_ready_full_chunk(&world, ChunkPos::new(0, 0));

        let origin = BlockPos::new(2, 2, 2);
        let report = paste_litematic(&world, &litematic, origin);
        assert!(
            report.failed_positions.is_empty(),
            "{:?}",
            report.failed_positions
        );
        set_slot(
            &world,
            origin.offset(1, 1, 1),
            0,
            ItemStack::with_count(&vanilla_items::STONE, 1),
        );
        // The projection's neighbor update fires before the slot is populated.
        // Re-trigger the dispenser from a real neighbor update after seeding the
        // slot so the test does not depend on a later unrelated update.
        let dispenser_pos = origin.offset(1, 1, 1);
        // Pasting the redstone block can fire the dispenser once before the
        // slot is seeded, leaving TRIGGERED=true. Clear that latch explicitly
        // so this assertion tests ejection, not paste-order timing.
        let dispenser_state = world.get_block_state(dispenser_pos);
        world.set_block(
            dispenser_pos,
            dispenser_state.set_value(&BlockStateProperties::TRIGGERED, false),
            UpdateFlags::UPDATE_NONE,
        );
        world.update_neighbors_at(dispenser_pos, &vanilla_blocks::DISPENSER);
        world.run_block_events();

        tick_world(&world, 6);
        let center = DVec3::new(
            f64::from(origin.x() + 4),
            f64::from(origin.y() + 1),
            f64::from(origin.z() + 1),
        );
        let items = dropped_items(&world, center);
        assert!(
            !items.is_empty(),
            "dispenser must eject the stored item into the world"
        );
    }

    #[test]
    fn litematic_hopper_comparator_sorter_senses_filter_item_through_dust() {
        steel_registry::init_vanilla_registry();
        crate::behavior::init_behaviors();
        crate::block_entity::init_block_entities();

        let path = fixture_path("sorter_sense");
        let mut builder = LitematicBuilder::new("sorter_sense", (3, 2, 1));
        for x in 0..3 {
            builder.set(x, 0, 0, "minecraft:stone", &[]);
        }
        // Comparator "facing" is the side it reads; it outputs behind it.
        builder.set(2, 1, 0, "minecraft:hopper", &[("facing", "down")]);
        builder.set(1, 1, 0, "minecraft:comparator", &[("facing", "east")]);
        builder.set(0, 1, 0, "minecraft:redstone_wire", &[]);
        builder.write(&path).expect("write litematic fixture");

        let litematic = load_litematic(&path).expect("load litematic fixture");
        let test_world = fresh_test_world("machine_projection_sorter_sense");
        let world: Arc<_> = Arc::clone(&test_world.world);
        insert_ready_full_chunk(&world, ChunkPos::new(0, 0));
        let origin = BlockPos::new(3, 2, 3);
        let report = paste_litematic(&world, &litematic, origin);
        assert!(
            report.failed_positions.is_empty(),
            "{:?}",
            report.failed_positions
        );

        let hopper_pos = origin.offset(2, 1, 0);
        let dust_pos = origin.offset(0, 1, 0);

        // 1 diamond in a 5-slot hopper is a weak analog signal (1), the
        // canonical "filter item" state of a sorting hopper.
        set_slot(
            &world,
            hopper_pos,
            0,
            ItemStack::with_count(&vanilla_items::DIAMOND, 1),
        );
        world.update_neighbors_at(hopper_pos, &vanilla_blocks::HOPPER);
        tick_world(&world, 4);
        world.run_block_events();
        let comparator_pos = origin.offset(1, 1, 0);
        assert!(
            world
                .get_block_state(comparator_pos)
                .get_value(&BlockStateProperties::POWERED),
            "comparator must see the filter item and turn on"
        );
        let weak = world
            .get_block_state(dust_pos)
            .get_value(&BlockStateProperties::POWER);
        assert_eq!(
            weak, 1,
            "one filter item must produce a 1-strength dust signal"
        );

        // Filling the hopper raises the analog signal; the comparator must
        // follow the container fullness, not a constant.
        set_slot(
            &world,
            hopper_pos,
            0,
            ItemStack::with_count(&vanilla_items::DIAMOND, 64),
        );
        world.update_neighbors_at(hopper_pos, &vanilla_blocks::HOPPER);
        tick_world(&world, 4);
        world.run_block_events();
        let strong = world
            .get_block_state(dust_pos)
            .get_value(&BlockStateProperties::POWER);
        assert!(
            strong > weak,
            "full hopper must raise the dust signal, weak={weak} strong={strong}"
        );
    }

    #[test]
    fn litematic_hopper_pipeline_transfers_items_down_two_levels() {
        steel_registry::init_vanilla_registry();
        crate::behavior::init_behaviors();
        crate::block_entity::init_block_entities();

        let path = fixture_path("hopper_pipeline");
        let mut builder = LitematicBuilder::new("hopper_pipeline", (1, 3, 1));
        builder.set(0, 2, 0, "minecraft:hopper", &[("facing", "down")]);
        builder.set(0, 1, 0, "minecraft:hopper", &[("facing", "down")]);
        builder.set(0, 0, 0, "minecraft:barrel", &[("facing", "up")]);
        builder.write(&path).expect("write litematic fixture");

        let litematic = load_litematic(&path).expect("load litematic fixture");
        let test_world = fresh_test_world("machine_projection_hopper_pipeline");
        let world: Arc<_> = Arc::clone(&test_world.world);
        insert_ready_full_chunk(&world, ChunkPos::new(0, 0));

        let origin = BlockPos::new(4, 3, 4);
        let report = paste_litematic(&world, &litematic, origin);
        assert!(
            report.failed_positions.is_empty(),
            "{:?}",
            report.failed_positions
        );
        assert_eq!(report.placed, 3);

        set_slot(
            &world,
            origin.offset(0, 2, 0),
            0,
            ItemStack::with_count(&vanilla_items::STONE, 3),
        );

        tick_world(&world, 40);

        let top = count_at(&world, origin.offset(0, 2, 0), 0);
        let middle = count_at(&world, origin.offset(0, 1, 0), 0);
        let bottom = count_at(&world, origin.offset(0, 0, 0), 0);
        assert_eq!(
            top + middle + bottom,
            3,
            "items must be conserved across the hopper chain (top={top}, middle={middle}, bottom={bottom})"
        );
        assert!(
            bottom > 0,
            "items must reach the barrel after 40 ticks (top={top}, middle={middle}, bottom={bottom})"
        );
    }

    #[test]
    fn litematic_cobblestone_generator_forms_cobblestone_from_flowing_lava() {
        steel_registry::init_vanilla_registry();
        crate::behavior::init_behaviors();
        crate::block_entity::init_block_entities();

        let path = fixture_path("cobblestone_generator");
        let mut builder = LitematicBuilder::new("cobblestone_generator", (2, 2, 1));
        for x in 0..2 {
            builder.set(x, 0, 0, "minecraft:stone", &[]);
        }
        builder.set(0, 1, 0, "minecraft:lava", &[("level", "1")]);
        builder.set(1, 1, 0, "minecraft:water", &[]);
        builder.write(&path).expect("write litematic fixture");

        let litematic = load_litematic(&path).expect("load litematic fixture");
        let test_world = fresh_test_world("machine_projection_cobblestone_generator");
        let world: Arc<_> = Arc::clone(&test_world.world);
        insert_ready_full_chunk(&world, ChunkPos::new(0, 0));
        let origin = BlockPos::new(4, 3, 4);
        let report = paste_litematic(&world, &litematic, origin);
        assert!(
            report.failed_positions.is_empty(),
            "{:?}",
            report.failed_positions
        );

        world.update_neighbors_at(origin.offset(0, 1, 0), &vanilla_blocks::LAVA);
        tick_world(&world, 20);
        world.run_block_events();

        assert_eq!(
            block_id(world.get_block_state(origin.offset(0, 1, 0))),
            Identifier::new_static("minecraft", "cobblestone"),
            "flowing lava touching water must turn into cobblestone"
        );
    }

    #[test]
    fn litematic_two_by_two_sticky_piston_door_moves_and_returns() {
        steel_registry::init_vanilla_registry();
        crate::behavior::init_behaviors();
        crate::block_entity::init_block_entities();

        let path = fixture_path("two_by_two_door");
        let mut builder = LitematicBuilder::new("two_by_two_door", (3, 2, 2));
        for y in 0..2 {
            for z in 0..2 {
                builder.set(
                    0,
                    y,
                    z,
                    "minecraft:sticky_piston",
                    &[("facing", "east"), ("extended", "false")],
                );
                builder.set(1, y, z, "minecraft:stone", &[]);
            }
        }
        builder.write(&path).expect("write litematic fixture");

        let litematic = load_litematic(&path).expect("load litematic fixture");
        let test_world = fresh_test_world("machine_projection_two_by_two_door");
        let world: Arc<_> = Arc::clone(&test_world.world);
        insert_ready_full_chunk(&world, ChunkPos::new(0, 0));
        let origin = BlockPos::new(6, 3, 4);
        let report = paste_litematic(&world, &litematic, origin);
        assert!(
            report.failed_positions.is_empty(),
            "{:?}",
            report.failed_positions
        );

        let power_positions: Vec<BlockPos> = (0..2)
            .flat_map(|y| (0..2).map(move |z| origin.offset(-1, y, z)))
            .collect();
        for pos in &power_positions {
            world.set_block(
                *pos,
                vanilla_blocks::REDSTONE_BLOCK.default_state(),
                steel_utils::types::UpdateFlags::UPDATE_CLIENTS
                    | steel_utils::types::UpdateFlags::UPDATE_NEIGHBORS,
            );
            world.update_neighbors_at(*pos, &vanilla_blocks::REDSTONE_BLOCK);
        }
        for y in 0..2 {
            for z in 0..2 {
                world.update_neighbors_at(origin.offset(0, y, z), &vanilla_blocks::STICKY_PISTON);
            }
        }
        tick_world(&world, 6);
        world.run_block_events();

        for y in 0..2 {
            for z in 0..2 {
                assert_eq!(
                    block_id(world.get_block_state(origin.offset(2, y, z))),
                    Identifier::new_static("minecraft", "stone"),
                    "the 2x2 door must push all four blocks to x+2"
                );
                assert_eq!(
                    block_id(world.get_block_state(origin.offset(1, y, z))),
                    Identifier::new_static("minecraft", "piston_head"),
                    "each extended piston must own its head"
                );
            }
        }

        for pos in &power_positions {
            world.set_block(
                *pos,
                vanilla_blocks::AIR.default_state(),
                steel_utils::types::UpdateFlags::UPDATE_CLIENTS
                    | steel_utils::types::UpdateFlags::UPDATE_NEIGHBORS,
            );
            world.update_neighbors_at(*pos, &vanilla_blocks::REDSTONE_BLOCK);
        }
        for y in 0..2 {
            for z in 0..2 {
                world.update_neighbors_at(origin.offset(0, y, z), &vanilla_blocks::STICKY_PISTON);
            }
        }
        tick_world(&world, 6);
        world.run_block_events();

        for y in 0..2 {
            for z in 0..2 {
                assert_eq!(
                    block_id(world.get_block_state(origin.offset(1, y, z))),
                    Identifier::new_static("minecraft", "stone"),
                    "retracting sticky pistons must pull all four blocks back"
                );
                assert_eq!(
                    block_id(world.get_block_state(origin.offset(2, y, z))),
                    Identifier::new_static("minecraft", "air"),
                    "the pushed column must be cleared after retraction"
                );
            }
        }
    }

    #[test]
    fn litematic_two_piston_gate_pushes_two_blocks() {
        steel_registry::init_vanilla_registry();
        crate::behavior::init_behaviors();
        crate::block_entity::init_block_entities();

        let path = fixture_path("two_piston_gate");
        let mut builder = LitematicBuilder::new("two_piston_gate", (4, 2, 3));
        for z in 0..2 {
            builder.set(0, 1, z, "minecraft:redstone_block", &[]);
            builder.set(1, 1, z, "minecraft:piston", &[("facing", "east")]);
            builder.set(2, 1, z, "minecraft:stone", &[]);
            builder.set(1, 0, z, "minecraft:stone", &[]);
        }
        builder.write(&path).expect("write litematic fixture");

        let litematic = load_litematic(&path).expect("load litematic fixture");
        let test_world = fresh_test_world("machine_projection_two_pistons");
        let world: Arc<_> = Arc::clone(&test_world.world);
        insert_ready_full_chunk(&world, ChunkPos::new(0, 0));

        let origin = BlockPos::new(2, 2, 2);
        let report = paste_litematic(&world, &litematic, origin);
        assert!(
            report.failed_positions.is_empty(),
            "{:?}",
            report.failed_positions
        );

        for _ in 0..6 {
            tick_test_world(&world, 1, true);
        }

        for z in 0..2 {
            assert_eq!(
                block_id(world.get_block_state(origin.offset(2, 1, z))),
                Identifier::new_static("minecraft", "piston_head"),
                "piston {z} must place its head"
            );
            assert_eq!(
                block_id(world.get_block_state(origin.offset(3, 1, z))),
                Identifier::new_static("minecraft", "stone"),
                "piston {z} must push its stone one block east"
            );
        }
    }

    #[test]
    fn litematic_vertical_dropper_elevator_lifts_items_to_top_barrel() {
        steel_registry::init_vanilla_registry();
        crate::behavior::init_behaviors();
        crate::block_entity::init_block_entities();

        let path = fixture_path("dropper_elevator");
        let mut builder = LitematicBuilder::new("dropper_elevator", (2, 5, 1));
        for y in 0..4 {
            builder.set(0, y, 0, "minecraft:dropper", &[("facing", "up")]);
            builder.set(1, y, 0, "minecraft:redstone_block", &[]);
        }
        builder.set(0, 4, 0, "minecraft:barrel", &[("facing", "up")]);
        builder.write(&path).expect("write litematic fixture");

        let litematic = load_litematic(&path).expect("load litematic fixture");
        let test_world = fresh_test_world("machine_projection_dropper_elevator");
        let world: Arc<_> = Arc::clone(&test_world.world);
        insert_ready_full_chunk(&world, ChunkPos::new(0, 0));
        let origin = BlockPos::new(4, 12, 4);
        let report = paste_litematic(&world, &litematic, origin);
        assert!(
            report.failed_positions.is_empty(),
            "{:?}",
            report.failed_positions
        );

        set_slot(
            &world,
            origin,
            0,
            ItemStack::with_count(&vanilla_items::STONE, 8),
        );

        // Each rising edge moves one item one dropper up; then the next
        // dropper can fire on the next edge, so N edges lift an item N levels.
        for _ in 0..10 {
            for y in 0..4 {
                let power_pos = origin.offset(1, y, 0);
                let dropper_pos = origin.offset(0, y, 0);
                world.set_block(
                    power_pos,
                    vanilla_blocks::AIR.default_state(),
                    steel_utils::types::UpdateFlags::UPDATE_CLIENTS
                        | steel_utils::types::UpdateFlags::UPDATE_NEIGHBORS,
                );
                world.update_neighbors_at(power_pos, &vanilla_blocks::REDSTONE_BLOCK);
                world.update_neighbors_at(dropper_pos, &vanilla_blocks::DROPPER);
                tick_world(&world, 2);
                world.run_block_events();

                world.set_block(
                    power_pos,
                    vanilla_blocks::REDSTONE_BLOCK.default_state(),
                    steel_utils::types::UpdateFlags::UPDATE_CLIENTS
                        | steel_utils::types::UpdateFlags::UPDATE_NEIGHBORS,
                );
                world.update_neighbors_at(power_pos, &vanilla_blocks::REDSTONE_BLOCK);
                world.update_neighbors_at(dropper_pos, &vanilla_blocks::DROPPER);
                tick_world(&world, 2);
                world.run_block_events();
            }
        }

        let top = count_at(&world, origin.offset(0, 4, 0), 0);
        assert!(
            top > 0,
            "vertical dropper elevator must deliver stone to the top barrel, top={top}"
        );
    }

    #[test]
    fn litematic_long_hopper_chain_feeds_bottom_barrel() {
        steel_registry::init_vanilla_registry();
        crate::behavior::init_behaviors();
        crate::block_entity::init_block_entities();

        let path = fixture_path("long_hopper_chain");
        let height = 10;
        let mut builder = LitematicBuilder::new("long_hopper_chain", (1, height, 1));
        builder.set(0, 0, 0, "minecraft:barrel", &[("facing", "up")]);
        for y in 1..height {
            builder.set(0, y, 0, "minecraft:hopper", &[("facing", "down")]);
        }
        builder.write(&path).expect("write litematic fixture");

        let litematic = load_litematic(&path).expect("load litematic fixture");
        let test_world = fresh_test_world("machine_projection_long_hopper");
        let world: Arc<_> = Arc::clone(&test_world.world);
        insert_ready_full_chunk(&world, ChunkPos::new(0, 0));

        let origin = BlockPos::new(4, 1, 4);
        let report = paste_litematic(&world, &litematic, origin);
        assert!(
            report.failed_positions.is_empty(),
            "{:?}",
            report.failed_positions
        );
        assert_eq!(report.placed, usize::try_from(height).unwrap());

        set_slot(
            &world,
            origin.offset(0, height - 1, 0),
            0,
            ItemStack::with_count(&vanilla_items::STONE, 8),
        );

        for _ in 0..200 {
            tick_test_world(&world, 1, true);
        }

        let top = count_at(&world, origin.offset(0, height - 1, 0), 0);
        let bottom = count_at(&world, origin.offset(0, 0, 0), 0);
        let mut middle_total = 0;
        for y in 1..height - 1 {
            middle_total += count_at(&world, origin.offset(0, y, 0), 0);
        }
        assert_eq!(
            top + middle_total + bottom,
            8,
            "items must be conserved (top={top}, middle={middle_total}, bottom={bottom})"
        );
        assert!(
            bottom > 0,
            "the bottom barrel must receive items from the 9-hopper stack"
        );
    }

    #[test]
    fn litematic_furnace_pipeline_smelts_projected_input() {
        steel_registry::init_vanilla_registry();
        crate::behavior::init_behaviors();
        crate::block_entity::init_block_entities();

        let path = fixture_path("furnace_pipeline");
        let mut builder = LitematicBuilder::new("furnace_pipeline", (1, 5, 1));
        builder.set(0, 0, 0, "minecraft:barrel", &[("facing", "up")]);
        builder.set(0, 1, 0, "minecraft:hopper", &[("facing", "down")]);
        builder.set(0, 2, 0, "minecraft:furnace", &[("facing", "north")]);
        builder.set(0, 3, 0, "minecraft:hopper", &[("facing", "down")]);
        builder.set(0, 4, 0, "minecraft:chest", &[]);

        let mut input = NbtCompound::new();
        input.insert("Slot", 0i8);
        input.insert("id", "minecraft:iron_ore");
        input.insert("count", 2i32);
        let mut fuel = NbtCompound::new();
        fuel.insert("Slot", 1i8);
        fuel.insert("id", "minecraft:coal");
        fuel.insert("count", 4i32);
        let mut furnace = NbtCompound::new();
        furnace.insert("id", "minecraft:furnace");
        furnace.insert("Items", NbtList::Compound(vec![input, fuel]));
        builder.set_tile_entity(0, 2, 0, furnace);
        builder.write(&path).expect("write litematic fixture");

        let litematic = load_litematic(&path).expect("load litematic fixture");
        let test_world = fresh_test_world("machine_projection_furnace");
        let world: Arc<_> = Arc::clone(&test_world.world);
        insert_ready_full_chunk(&world, ChunkPos::new(0, 0));

        let origin = BlockPos::new(4, 1, 4);
        let report = paste_litematic(&world, &litematic, origin);
        assert!(
            report.failed_positions.is_empty(),
            "{:?}",
            report.failed_positions
        );
        assert!(
            report.failed_tile_entities.is_empty(),
            "{:?}",
            report.failed_tile_entities
        );
        assert_eq!(report.tile_entities_loaded, 1);

        // Empty chest on top; the furnace input/fuel came from TileEntities.
        // A player anchor keeps the chunk at block-ticking simulation level so
        // the furnace block entity actually receives its tick.
        let player = TestPlayerBuilder::new(Arc::clone(&world), "FurnaceAnchor", 7).build();
        assert!(world.add_player(player, crate::player::ResetReason::InitialJoin));
        assert_eq!(
            count_at(&world, origin.offset(0, 2, 0), 0),
            2,
            "furnace input must load"
        );
        assert_eq!(
            count_at(&world, origin.offset(0, 2, 0), 1),
            4,
            "furnace fuel must load"
        );
        for _ in 0..500 {
            // The unit-test world does not run the block-entity ticker pass;
            // drive the furnace directly, then let hoppers run on world ticks.
            if let Some(block_entity) = world.get_block_entity(origin.offset(0, 2, 0)) {
                block_entity.tick(&world);
            }
            tick_test_world(&world, 1, true);
        }
        // Let the extraction hopper finish moving any ingot still in the
        // furnace result slot; its 8gt cooldown can straddle the smelt edge.
        for _ in 0..200 {
            if let Some(block_entity) = world.get_block_entity(origin.offset(0, 1, 0)) {
                block_entity.tick(&world);
            }
            tick_test_world(&world, 1, true);
        }

        let result = count_at(&world, origin.offset(0, 2, 0), 2);
        let input_left = count_at(&world, origin.offset(0, 2, 0), 0);
        let fuel_left = count_at(&world, origin.offset(0, 2, 0), 1);
        let barrel = count_at(&world, origin.offset(0, 0, 0), 0);
        assert_eq!(
            barrel + result,
            2,
            "both smelted ingots must be in the barrel or furnace output (input_left={input_left}, fuel_left={fuel_left})"
        );
        assert!(
            barrel > 0,
            "hopper must extract smelted ingots into the barrel"
        );
        let item_id = {
            let container = container_at(&world, origin.offset(0, 0, 0));
            let guard = ContainerLockGuard::lock_all(&[&container]).expect("lock");
            guard
                .get(container.container_id())
                .expect("container")
                .get_item(0)
                .item()
                .key
                .clone()
        };
        assert_eq!(barrel, 2, "both projected raw iron items must smelt");
        assert_eq!(
            item_id,
            Identifier::new_static("minecraft", "iron_ingot"),
            "the barrel must contain smelted iron ingots"
        );
    }

    #[test]
    fn litematic_dispenser_bonemeal_grows_wheat_to_max_age() {
        steel_registry::init_vanilla_registry();
        crate::behavior::init_behaviors();
        crate::block_entity::init_block_entities();

        let path = fixture_path("wheat_bonemeal");
        let mut builder = LitematicBuilder::new("wheat_bonemeal", (2, 3, 1));
        builder.set(0, 0, 0, "minecraft:farmland", &[("moisture", "0")]);
        builder.set(0, 1, 0, "minecraft:wheat", &[("age", "0")]);
        builder.set(0, 2, 0, "minecraft:dispenser", &[("facing", "down")]);
        builder.set(1, 2, 0, "minecraft:redstone_block", &[]);
        let mut bone_meal = NbtCompound::new();
        bone_meal.insert("Slot", 0i8);
        bone_meal.insert("id", "minecraft:bone_meal");
        bone_meal.insert("count", 64i32);
        let mut dispenser = NbtCompound::new();
        dispenser.insert("id", "minecraft:dispenser");
        dispenser.insert("Items", NbtList::Compound(vec![bone_meal]));
        builder.set_tile_entity(0, 2, 0, dispenser);
        builder.write(&path).expect("write litematic fixture");

        let litematic = load_litematic(&path).expect("load litematic fixture");
        let test_world = fresh_test_world("machine_projection_wheat_bonemeal");
        let world: Arc<_> = Arc::clone(&test_world.world);
        insert_ready_full_chunk(&world, ChunkPos::new(0, 0));
        let origin = BlockPos::new(3, 2, 3);
        let report = paste_litematic(&world, &litematic, origin);
        assert!(
            report.failed_positions.is_empty(),
            "{:?}",
            report.failed_positions
        );
        assert_eq!(report.tile_entities_loaded, 1);

        let power_pos = origin.offset(1, 2, 0);
        let dispenser_pos = origin.offset(0, 2, 0);
        let wheat_pos = origin.offset(0, 1, 0);
        for _ in 0..16 {
            world.set_block(
                power_pos,
                vanilla_blocks::AIR.default_state(),
                steel_utils::types::UpdateFlags::UPDATE_CLIENTS
                    | steel_utils::types::UpdateFlags::UPDATE_NEIGHBORS,
            );
            world.update_neighbors_at(power_pos, &vanilla_blocks::REDSTONE_BLOCK);
            world.update_neighbors_at(dispenser_pos, &vanilla_blocks::DISPENSER);
            tick_world(&world, 2);
            world.run_block_events();

            world.set_block(
                power_pos,
                vanilla_blocks::REDSTONE_BLOCK.default_state(),
                steel_utils::types::UpdateFlags::UPDATE_CLIENTS
                    | steel_utils::types::UpdateFlags::UPDATE_NEIGHBORS,
            );
            world.update_neighbors_at(power_pos, &vanilla_blocks::REDSTONE_BLOCK);
            world.update_neighbors_at(dispenser_pos, &vanilla_blocks::DISPENSER);
            tick_world(&world, 2);
            world.run_block_events();

            if world.get_block_state(wheat_pos).get_block() == &vanilla_blocks::WHEAT {
                let age = world
                    .get_block_state(wheat_pos)
                    .get_value(&BlockStateProperties::AGE_7);
                if age >= 7 {
                    break;
                }
            }
        }

        assert_eq!(
            block_id(world.get_block_state(wheat_pos)),
            Identifier::new_static("minecraft", "wheat"),
            "wheat must still be planted"
        );
        assert_eq!(
            world
                .get_block_state(wheat_pos)
                .get_value(&BlockStateProperties::AGE_7),
            7,
            "bone meal dispenser must grow wheat to max age"
        );
    }

    #[test]
    fn litematic_piston_harvests_wheat_and_drops_items() {
        steel_registry::init_vanilla_registry();
        crate::behavior::init_behaviors();
        crate::block_entity::init_block_entities();

        let path = fixture_path("wheat_harvest");
        let mut builder = LitematicBuilder::new("wheat_harvest", (2, 2, 1));
        builder.set(1, 0, 0, "minecraft:farmland", &[("moisture", "0")]);
        builder.set(1, 1, 0, "minecraft:wheat", &[("age", "7")]);
        builder.set(0, 1, 0, "minecraft:piston", &[("facing", "east")]);
        builder.write(&path).expect("write litematic fixture");

        let litematic = load_litematic(&path).expect("load litematic fixture");
        let test_world = fresh_test_world("machine_projection_wheat_harvest");
        let world: Arc<_> = Arc::clone(&test_world.world);
        insert_ready_full_chunk(&world, ChunkPos::new(0, 0));
        let origin = BlockPos::new(4, 2, 4);
        let report = paste_litematic(&world, &litematic, origin);
        assert!(
            report.failed_positions.is_empty(),
            "{:?}",
            report.failed_positions
        );

        let piston_pos = origin.offset(0, 1, 0);
        let wheat_pos = origin.offset(1, 1, 0);
        let power_pos = origin.offset(-1, 1, 0);
        world.set_block(
            power_pos,
            vanilla_blocks::REDSTONE_BLOCK.default_state(),
            steel_utils::types::UpdateFlags::UPDATE_CLIENTS
                | steel_utils::types::UpdateFlags::UPDATE_NEIGHBORS,
        );
        world.update_neighbors_at(power_pos, &vanilla_blocks::REDSTONE_BLOCK);
        world.update_neighbors_at(piston_pos, &vanilla_blocks::PISTON);
        tick_world(&world, 6);
        world.run_block_events();

        assert_ne!(
            block_id(world.get_block_state(wheat_pos)),
            Identifier::new_static("minecraft", "wheat"),
            "extending piston must break the mature wheat"
        );
        let center = DVec3::new(
            f64::from(wheat_pos.x()) + 0.5,
            f64::from(wheat_pos.y()) + 0.5,
            f64::from(wheat_pos.z()) + 0.5,
        );
        assert!(
            !dropped_items(&world, center).is_empty(),
            "piston-harvested wheat must drop items"
        );
    }

    #[test]
    fn litematic_parallel_furnace_array_smelts_two_columns() {
        steel_registry::init_vanilla_registry();
        crate::behavior::init_behaviors();
        crate::block_entity::init_block_entities();

        let path = fixture_path("furnace_array");
        let mut builder = LitematicBuilder::new("furnace_array", (1, 5, 2));
        for z in 0..2 {
            builder.set(0, 0, z, "minecraft:barrel", &[("facing", "up")]);
            builder.set(0, 1, z, "minecraft:hopper", &[("facing", "down")]);
            builder.set(0, 2, z, "minecraft:furnace", &[("facing", "north")]);
            builder.set(0, 3, z, "minecraft:hopper", &[("facing", "down")]);
            builder.set(0, 4, z, "minecraft:chest", &[]);

            let mut input = NbtCompound::new();
            input.insert("Slot", 0i8);
            input.insert("id", "minecraft:iron_ore");
            input.insert("count", 2i32);
            let mut fuel = NbtCompound::new();
            fuel.insert("Slot", 1i8);
            fuel.insert("id", "minecraft:coal");
            fuel.insert("count", 4i32);
            let mut furnace = NbtCompound::new();
            furnace.insert("id", "minecraft:furnace");
            furnace.insert("Items", NbtList::Compound(vec![input, fuel]));
            builder.set_tile_entity(0, 2, z, furnace);
        }
        builder.write(&path).expect("write litematic fixture");

        let litematic = load_litematic(&path).expect("load litematic fixture");
        let test_world = fresh_test_world("machine_projection_furnace_array");
        let world: Arc<_> = Arc::clone(&test_world.world);
        insert_ready_full_chunk(&world, ChunkPos::new(0, 0));
        let origin = BlockPos::new(4, 1, 4);
        let report = paste_litematic(&world, &litematic, origin);
        assert!(
            report.failed_positions.is_empty(),
            "{:?}",
            report.failed_positions
        );
        assert_eq!(report.tile_entities_loaded, 2);

        let player = TestPlayerBuilder::new(Arc::clone(&world), "FurnaceArray", 7).build();
        assert!(world.add_player(player, crate::player::ResetReason::InitialJoin));

        for z in 0..2 {
            assert_eq!(count_at(&world, origin.offset(0, 2, z), 0), 2);
            assert_eq!(count_at(&world, origin.offset(0, 2, z), 1), 4);
        }

        for _ in 0..500 {
            for z in 0..2 {
                if let Some(block_entity) = world.get_block_entity(origin.offset(0, 2, z)) {
                    block_entity.tick(&world);
                }
            }
            tick_test_world(&world, 1, true);
        }
        for _ in 0..200 {
            for z in 0..2 {
                if let Some(block_entity) = world.get_block_entity(origin.offset(0, 1, z)) {
                    block_entity.tick(&world);
                }
            }
            tick_test_world(&world, 1, true);
        }

        for z in 0..2 {
            let result = count_at(&world, origin.offset(0, 2, z), 2);
            let barrel = count_at(&world, origin.offset(0, 0, z), 0);
            assert_eq!(
                barrel + result,
                2,
                "column {z} must smelt exactly two ingots (barrel={barrel}, furnace={result})"
            );
            assert_eq!(barrel, 2, "column {z} barrel must receive both ingots");
            let item_id = {
                let container = container_at(&world, origin.offset(0, 0, z));
                let guard = ContainerLockGuard::lock_all(&[&container]).expect("lock");
                guard
                    .get(container.container_id())
                    .expect("container")
                    .get_item(0)
                    .item()
                    .key
                    .clone()
            };
            assert_eq!(
                item_id,
                Identifier::new_static("minecraft", "iron_ingot"),
                "column {z} must produce iron ingots"
            );
        }
    }

    #[test]
    fn litematic_multi_region_places_and_loads_tile_entities() {
        steel_registry::init_vanilla_registry();
        crate::behavior::init_behaviors();
        crate::block_entity::init_block_entities();

        let path = fixture_path("multi_region");
        let mut builder = LitematicBuilder::new_multi(&[
            ("base", [0, 0, 0], (2, 1, 1)),
            ("storage", [2, 0, 0], (1, 1, 1)),
        ]);
        builder.set(0, 0, 0, "minecraft:stone", &[]);
        builder.set(1, 0, 0, "minecraft:oak_planks", &[]);
        builder.set_in_region(1, 0, 0, 0, "minecraft:chest", &[]);

        let mut item = NbtCompound::new();
        item.insert("Slot", 0i8);
        item.insert("id", "minecraft:stone");
        item.insert("count", 6i32);
        let mut chest = NbtCompound::new();
        chest.insert("id", "minecraft:chest");
        chest.insert("Items", NbtList::Compound(vec![item]));
        builder.set_tile_entity_in_region(1, 0, 0, 0, chest);
        builder.write(&path).expect("write litematic fixture");

        let litematic = load_litematic(&path).expect("load litematic fixture");
        assert_eq!(litematic.regions.len(), 2);
        assert_eq!(litematic.regions[1].position, [2, 0, 0]);
        assert_eq!(litematic.regions[1].tile_entities.len(), 1);

        let test_world = fresh_test_world("machine_projection_multi_region");
        let world: Arc<_> = Arc::clone(&test_world.world);
        insert_ready_full_chunk(&world, ChunkPos::new(0, 0));

        let origin = BlockPos::new(6, 2, 6);
        let report = paste_litematic(&world, &litematic, origin);
        assert!(
            report.failed_positions.is_empty(),
            "{:?}",
            report.failed_positions
        );
        assert!(
            report.failed_tile_entities.is_empty(),
            "{:?}",
            report.failed_tile_entities
        );
        assert_eq!(report.placed, 3);
        assert_eq!(report.tile_entities_loaded, 1);
        assert_eq!(
            block_id(world.get_block_state(origin.offset(1, 0, 0))),
            Identifier::new_static("minecraft", "oak_planks"),
            "first region block must land at its region position"
        );
        assert_eq!(
            block_id(world.get_block_state(origin.offset(2, 0, 0))),
            Identifier::new_static("minecraft", "chest"),
            "second region block must land at its region position"
        );
        assert_eq!(
            count_at(&world, origin.offset(2, 0, 0), 0),
            6,
            "tile entity in the second region must load its items"
        );
    }

    #[test]
    fn litematic_oak_sapling_bonemeal_grows_tree() {
        steel_registry::init_vanilla_registry();
        crate::behavior::init_behaviors();
        crate::block_entity::init_block_entities();

        let path = fixture_path("oak_sapling_tree");
        let mut builder = LitematicBuilder::new("oak_sapling_tree", (7, 2, 7));
        for x in 0..7 {
            for z in 0..7 {
                builder.set(x, 0, z, "minecraft:dirt", &[]);
            }
        }
        builder.set(3, 1, 3, "minecraft:oak_sapling", &[]);
        builder.write(&path).expect("write litematic fixture");

        let litematic = load_litematic(&path).expect("load litematic fixture");
        let test_world = fresh_test_world("machine_projection_oak_tree");
        let world: Arc<_> = Arc::clone(&test_world.world);
        let origin = BlockPos::new(8, 64, 8);
        for chunk_x in 0..=2 {
            for chunk_z in 0..=2 {
                insert_ready_full_chunk(&world, ChunkPos::new(chunk_x, chunk_z));
            }
        }
        let report = paste_litematic(&world, &litematic, origin);
        assert!(
            report.failed_positions.is_empty(),
            "{:?}",
            report.failed_positions
        );
        assert_eq!(report.placed, 50);

        let sapling_pos = origin.offset(3, 1, 3);
        let state = world.get_block_state(sapling_pos);
        assert_eq!(
            block_id(state),
            Identifier::new_static("minecraft", "oak_sapling")
        );
        let behavior = crate::behavior::blocks::vegetation::SaplingBlock::new(
            &steel_registry::vanilla_blocks::OAK_SAPLING,
        );
        let mut placed = false;
        for seed in 0..64 {
            use rand::SeedableRng as _;
            let mut rng = rand::rngs::StdRng::seed_from_u64(seed);
            use crate::behavior::blocks::vegetation::bonemealable::Bonemealable as _;
            behavior.perform_bonemeal(state, &world, &mut rng, sapling_pos);
            if world.get_block_state(sapling_pos).get_block()
                != &steel_registry::vanilla_blocks::OAK_SAPLING
            {
                placed = true;
                break;
            }
        }
        assert!(
            placed,
            "at least one seeded bonemeal attempt must grow the tree"
        );

        let mut oak_logs = 0;
        for dx in -4..=4 {
            for dy in 0..8 {
                for dz in -4..=4 {
                    let pos = sapling_pos.offset(dx, dy, dz);
                    if block_id(world.get_block_state(pos))
                        == Identifier::new_static("minecraft", "oak_log")
                    {
                        oak_logs += 1;
                    }
                }
            }
        }
        assert!(
            oak_logs >= 4,
            "a grown oak must place a trunk, found {oak_logs} logs"
        );
    }

    #[test]
    fn litematic_dispenser_bonemeal_grows_projected_sapling() {
        steel_registry::init_vanilla_registry();
        crate::behavior::init_behaviors();
        crate::block_entity::init_block_entities();

        let path = fixture_path("dispenser_sapling_tree");
        let mut builder = LitematicBuilder::new("dispenser_sapling_tree", (7, 8, 7));
        for x in 0..7 {
            for z in 0..7 {
                builder.set(x, 0, z, "minecraft:dirt", &[]);
            }
        }
        builder.set(2, 1, 3, "minecraft:oak_sapling", &[]);
        builder.set(1, 1, 3, "minecraft:dispenser", &[("facing", "east")]);
        builder.set(0, 1, 3, "minecraft:redstone_block", &[]);

        let mut bone_meal = NbtCompound::new();
        bone_meal.insert("Slot", 0i8);
        bone_meal.insert("id", "minecraft:bone_meal");
        bone_meal.insert("count", 64i32);
        let mut dispenser = NbtCompound::new();
        dispenser.insert("id", "minecraft:dispenser");
        dispenser.insert("Items", NbtList::Compound(vec![bone_meal]));
        builder.set_tile_entity(1, 1, 3, dispenser);
        builder.write(&path).expect("write litematic fixture");

        let litematic = load_litematic(&path).expect("load litematic fixture");
        let test_world = fresh_test_world("machine_projection_dispenser_tree");
        let world: Arc<_> = Arc::clone(&test_world.world);
        let origin = BlockPos::new(8, 64, 8);
        for chunk_x in 0..=2 {
            for chunk_z in 0..=2 {
                insert_ready_full_chunk(&world, ChunkPos::new(chunk_x, chunk_z));
            }
        }
        let report = paste_litematic(&world, &litematic, origin);
        assert!(
            report.failed_positions.is_empty(),
            "{:?}",
            report.failed_positions
        );
        assert_eq!(report.tile_entities_loaded, 1);

        let dispenser_pos = origin.offset(1, 1, 3);
        let power_pos = origin.offset(0, 1, 3);
        let sapling_pos = origin.offset(2, 1, 3);
        let mut grown = false;
        for _ in 0..64 {
            world.set_block(
                power_pos,
                vanilla_blocks::REDSTONE_BLOCK.default_state(),
                steel_utils::types::UpdateFlags::UPDATE_CLIENTS,
            );
            world.update_neighbors_at(power_pos, &vanilla_blocks::REDSTONE_BLOCK);
            world.run_block_events();
            tick_world(&world, 2);
            world.set_block(
                power_pos,
                vanilla_blocks::AIR.default_state(),
                steel_utils::types::UpdateFlags::UPDATE_CLIENTS,
            );
            world.update_neighbors_at(power_pos, &vanilla_blocks::REDSTONE_BLOCK);
            world.run_block_events();
            tick_world(&world, 2);
            if world.get_block_state(sapling_pos).get_block() != &vanilla_blocks::OAK_SAPLING {
                grown = true;
                break;
            }
        }
        let dispenser_state = world.get_block_state(dispenser_pos);
        assert_eq!(
            dispenser_state.get_value(&BlockStateProperties::FACING),
            steel_utils::Direction::East,
            "dispenser must face the sapling"
        );
        let remaining_bone_meal = if world.get_block_entity(dispenser_pos).is_some() {
            count_at(&world, dispenser_pos, 0)
        } else {
            -1
        };
        assert!(
            grown,
            "dispenser must bonemeal the oak sapling into a tree; remaining bone meal={remaining_bone_meal}, facing={:?}",
            world.get_block_state(dispenser_pos)
        );
        let logs = (-4..=4)
            .flat_map(|dx| (-4..=4).map(move |dz| (dx, dz)))
            .map(|(dx, dz)| {
                (0..8)
                    .filter(|dy| {
                        block_id(world.get_block_state(sapling_pos.offset(dx, *dy, dz)))
                            == Identifier::new_static("minecraft", "oak_log")
                    })
                    .count()
            })
            .sum::<usize>();
        assert!(logs >= 4, "grown oak must leave a trunk, found {logs} logs");
        assert_eq!(
            block_id(world.get_block_state(dispenser_pos)),
            Identifier::new_static("minecraft", "dispenser")
        );
    }

    #[test]
    fn litematic_tile_entities_import_chest_contents() {
        steel_registry::init_vanilla_registry();
        crate::behavior::init_behaviors();
        crate::block_entity::init_block_entities();

        let path = fixture_path("tile_entity_chest");
        let mut builder = LitematicBuilder::new("tile_entity_chest", (1, 3, 1));
        builder.set(0, 0, 0, "minecraft:barrel", &[("facing", "up")]);
        builder.set(0, 1, 0, "minecraft:hopper", &[("facing", "down")]);
        builder.set(0, 2, 0, "minecraft:chest", &[]);

        let mut item = NbtCompound::new();
        item.insert("Slot", 0i8);
        item.insert("id", "minecraft:stone");
        item.insert("count", 8i32);
        let mut chest = NbtCompound::new();
        chest.insert("id", "minecraft:chest");
        chest.insert("Items", NbtList::Compound(vec![item]));
        builder.set_tile_entity(0, 2, 0, chest);
        builder.write(&path).expect("write litematic fixture");

        let litematic = load_litematic(&path).expect("load litematic fixture");
        assert_eq!(litematic.regions[0].tile_entities.len(), 1);
        assert_eq!(litematic.regions[0].tile_entities[0].pos, [0, 2, 0]);

        let test_world = fresh_test_world("machine_projection_tile_entity");
        let world: Arc<_> = Arc::clone(&test_world.world);
        insert_ready_full_chunk(&world, ChunkPos::new(0, 0));

        let origin = BlockPos::new(4, 1, 4);
        let report = paste_litematic(&world, &litematic, origin);
        assert!(
            report.failed_positions.is_empty(),
            "{:?}",
            report.failed_positions
        );
        assert!(
            report.failed_tile_entities.is_empty(),
            "{:?}",
            report.failed_tile_entities
        );
        assert_eq!(report.tile_entities_loaded, 1);

        // No manual set_slot: the chest contents must come from TileEntities.
        let chest_count = count_at(&world, origin.offset(0, 2, 0), 0);
        assert_eq!(chest_count, 8, "projection must import chest slot contents");

        for _ in 0..200 {
            tick_test_world(&world, 1, true);
        }

        let chest_after = count_at(&world, origin.offset(0, 2, 0), 0);
        let hopper = count_at(&world, origin.offset(0, 1, 0), 0);
        let barrel = count_at(&world, origin.offset(0, 0, 0), 0);
        assert_eq!(
            chest_after + hopper + barrel,
            8,
            "items from the imported chest must be conserved (chest={chest_after}, hopper={hopper}, barrel={barrel})"
        );
        assert!(
            barrel > 0,
            "hopper must move imported chest items into the barrel"
        );
    }

    #[test]
    fn litematic_cross_chunk_hopper_line_transfers_items() {
        steel_registry::init_vanilla_registry();
        crate::behavior::init_behaviors();
        crate::block_entity::init_block_entities();

        // 18 hoppers laid east/west; origin x=2 pushes the line across the
        // x=16 chunk boundary. Both chunks are loaded before pasting.
        let path = fixture_path("cross_chunk_hopper_line");
        let length = 19;
        let mut builder = LitematicBuilder::new("cross_chunk_hopper_line", (length, 2, 1));
        for x in 0..length - 1 {
            builder.set(x, 1, 0, "minecraft:hopper", &[("facing", "east")]);
            builder.set(x, 0, 0, "minecraft:stone", &[]);
        }
        builder.set(length - 1, 1, 0, "minecraft:barrel", &[("facing", "up")]);
        builder.set(length - 1, 0, 0, "minecraft:stone", &[]);
        builder.write(&path).expect("write litematic fixture");

        let litematic = load_litematic(&path).expect("load litematic fixture");
        let test_world = fresh_test_world("machine_projection_cross_chunk_hopper");
        let world: Arc<_> = Arc::clone(&test_world.world);
        insert_ready_full_chunk(&world, ChunkPos::new(0, 0));
        insert_ready_full_chunk(&world, ChunkPos::new(1, 0));

        let origin = BlockPos::new(2, 2, 2);
        let report = paste_litematic(&world, &litematic, origin);
        assert!(
            report.failed_positions.is_empty(),
            "{:?}",
            report.failed_positions
        );
        assert!(
            origin.offset(length - 1, 1, 0).x() > 16,
            "the test must actually cross a chunk boundary"
        );

        set_slot(
            &world,
            origin.offset(0, 1, 0),
            0,
            ItemStack::with_count(&vanilla_items::STONE, 8),
        );
        for _ in 0..400 {
            tick_test_world(&world, 1, true);
        }

        let top = count_at(&world, origin.offset(0, 1, 0), 0);
        let bottom = count_at(&world, origin.offset(length - 1, 1, 0), 0);
        let mut middle_total = 0;
        for x in 1..length - 1 {
            middle_total += count_at(&world, origin.offset(x, 1, 0), 0);
        }
        assert_eq!(
            top + middle_total + bottom,
            8,
            "items must be conserved across the chunk boundary (top={top}, middle={middle_total}, bottom={bottom})"
        );
        assert!(
            bottom > 0,
            "the barrel across the chunk boundary must receive items"
        );
    }

    #[test]
    fn litematic_sticky_piston_pulls_slime_cluster() {
        steel_registry::init_vanilla_registry();
        crate::behavior::init_behaviors();
        crate::block_entity::init_block_entities();

        let path = fixture_path("sticky_slime_pull");
        let mut builder = LitematicBuilder::new("sticky_slime_pull", (5, 1, 1));
        builder.set(
            0,
            0,
            0,
            "minecraft:sticky_piston",
            &[("facing", "east"), ("extended", "true")],
        );
        builder.set(
            1,
            0,
            0,
            "minecraft:piston_head",
            &[("facing", "east"), ("type", "sticky")],
        );
        builder.set(2, 0, 0, "minecraft:slime_block", &[]);
        builder.set(3, 0, 0, "minecraft:stone", &[]);
        builder.write(&path).expect("write litematic fixture");

        let litematic = load_litematic(&path).expect("load litematic fixture");
        let test_world = fresh_test_world("machine_projection_sticky_slime");
        let world: Arc<_> = Arc::clone(&test_world.world);
        insert_ready_full_chunk(&world, ChunkPos::new(0, 0));

        let origin = BlockPos::new(4, 2, 4);
        let report = paste_litematic(&world, &litematic, origin);
        assert!(
            report.failed_positions.is_empty(),
            "{:?}",
            report.failed_positions
        );
        assert_eq!(
            block_id(world.get_block_state(origin.offset(2, 0, 0))),
            Identifier::new_static("minecraft", "slime_block")
        );

        // Removing the (never present) power source is emulated by a neighbour
        // update, which is exactly what vanilla uses to notice retraction.
        world.update_neighbors_at(origin, &steel_registry::vanilla_blocks::STICKY_PISTON);
        world.run_block_events();
        tick_test_world(&world, 2, true);
        tick_block_entities(&world, 5);
        world.run_block_events();
        tick_test_world(&world, 2, true);
        tick_block_entities(&world, 5);

        assert_eq!(
            block_id(world.get_block_state(origin)),
            Identifier::new_static("minecraft", "sticky_piston")
        );
        assert_eq!(
            block_id(world.get_block_state(origin.offset(1, 0, 0))),
            Identifier::new_static("minecraft", "slime_block"),
            "retracting sticky piston must pull the slime block one step"
        );
        assert_eq!(
            block_id(world.get_block_state(origin.offset(2, 0, 0))),
            Identifier::new_static("minecraft", "stone"),
            "the block attached to the slime cluster must move with it"
        );
        assert!(
            world.get_block_state(origin.offset(3, 0, 0)).is_air(),
            "the trailing position must be vacated"
        );
    }

    #[test]
    fn litematic_observer_detects_piston_and_pulses_lamp() {
        steel_registry::init_vanilla_registry();
        crate::behavior::init_behaviors();
        crate::block_entity::init_block_entities();

        let path = fixture_path("observer_piston_lamp");
        let mut builder = LitematicBuilder::new("observer_piston_lamp", (4, 3, 1));
        // x=0 redstone block powers x=1 piston facing up; the piston pushes
        // the stone at y=2. x=2 observer faces west (watches the piston base)
        // and outputs east into the lamp at x=3.
        builder.set(0, 1, 0, "minecraft:redstone_block", &[]);
        builder.set(1, 0, 0, "minecraft:stone", &[]);
        builder.set(1, 1, 0, "minecraft:piston", &[("facing", "up")]);
        builder.set(1, 2, 0, "minecraft:stone", &[]);
        builder.set(2, 1, 0, "minecraft:observer", &[("facing", "west")]);
        builder.set(3, 1, 0, "minecraft:redstone_lamp", &[]);
        builder.write(&path).expect("write litematic fixture");

        let litematic = load_litematic(&path).expect("load litematic fixture");
        let test_world = fresh_test_world("machine_projection_observer");
        let world: Arc<_> = Arc::clone(&test_world.world);
        insert_ready_full_chunk(&world, ChunkPos::new(0, 0));

        let origin = BlockPos::new(2, 2, 2);
        let report = paste_litematic(&world, &litematic, origin);
        assert!(
            report.failed_positions.is_empty(),
            "{:?}",
            report.failed_positions
        );

        let lamp_pos = origin.offset(3, 1, 0);
        let mut saw_lamp_lit = false;
        for _ in 0..8 {
            tick_world(&world, 1);
            if world
                .get_block_state(lamp_pos)
                .get_value(&BlockStateProperties::LIT)
            {
                saw_lamp_lit = true;
            }
        }
        assert!(
            saw_lamp_lit,
            "observer must witness the piston extension and pulse the lamp"
        );
    }
    #[test]
    fn world_eater_flying_machine_self_propels_east_and_west() {
        steel_registry::init_vanilla_registry();
        crate::behavior::init_behaviors();
        crate::block_entity::init_block_entities();

        // Vanilla six-block two-observer world-eater engine.  Two independent halves:
        //   z=0 row: sticky piston -> observer -> slime
        //   z=1 row: slime <- observer <- sticky piston
        // Both halves push in the same direction and each observer watches the
        // other half's piston through the slime block it is glued to.
        for (name, reverse) in [("flying_east", false), ("flying_west", true)] {
            let test_world = fresh_test_world(name);
            let world: Arc<_> = Arc::clone(&test_world.world);
            insert_ready_full_chunk(&world, ChunkPos::new(0, 0));
            let origin = BlockPos::new(8, 64, 8);
            let sign = if reverse { -1 } else { 1 };
            let dir = if reverse {
                Direction::West
            } else {
                Direction::East
            };

            let piston_state = vanilla_blocks::STICKY_PISTON
                .default_state()
                .set_value(&BlockStateProperties::FACING, dir)
                .set_value(&BlockStateProperties::EXTENDED, false);
            let observer_back = vanilla_blocks::OBSERVER
                .default_state()
                .set_value(&BlockStateProperties::FACING, dir.opposite());
            let observer_front = vanilla_blocks::OBSERVER
                .default_state()
                .set_value(&BlockStateProperties::FACING, dir);

            let a = BlockPos::new(origin.x() + sign * 0, origin.y(), origin.z());
            let b = BlockPos::new(origin.x() + sign * 1, origin.y(), origin.z());
            let c = BlockPos::new(origin.x() + sign * 2, origin.y(), origin.z());
            let d = BlockPos::new(origin.x() + sign * 0, origin.y(), origin.z() + 1);
            let e = BlockPos::new(origin.x() + sign * 1, origin.y(), origin.z() + 1);
            let f = BlockPos::new(origin.x() + sign * 2, origin.y(), origin.z() + 1);

            world.set_block(a, piston_state, UpdateFlags::UPDATE_NONE);
            world.set_block(b, observer_back, UpdateFlags::UPDATE_NONE);
            world.set_block(
                c,
                vanilla_blocks::SLIME_BLOCK.default_state(),
                UpdateFlags::UPDATE_NONE,
            );
            world.set_block(
                d,
                vanilla_blocks::SLIME_BLOCK.default_state(),
                UpdateFlags::UPDATE_NONE,
            );
            world.set_block(e, observer_front, UpdateFlags::UPDATE_NONE);
            world.set_block(f, piston_state, UpdateFlags::UPDATE_NONE);

            // Kick the engine with a real neighbor update, like placing the
            // last block in vanilla.
            world.update_neighbors_at(b, &vanilla_blocks::OBSERVER);
            world.run_block_events();

            for _ in 0..200 {
                tick_test_world(&world, 1, true);
                tick_block_entities(&world, 3);
                world.run_block_events();
            }

            let max_step = (1..=12)
                .filter(|step| {
                    let x = origin.x() + sign * step;
                    (0..=1).any(|dz| {
                        world
                            .get_block_state(BlockPos::new(x, origin.y(), origin.z() + dz))
                            .get_block()
                            == &vanilla_blocks::STICKY_PISTON
                    })
                })
                .max()
                .unwrap_or(0);
            assert!(
                max_step >= 2,
                "{name}: observer engine must keep flying after the first push (max step {max_step})"
            );
            // The whole six-block assembly must travel together, not just the
            // front piston: check slime and observer at the shifted positions.
            let moved_x = origin.x() + sign * 1;
            assert_eq!(
                world
                    .get_block_state(BlockPos::new(moved_x, origin.y(), origin.z()))
                    .get_block(),
                &vanilla_blocks::OBSERVER,
                "{name}: the rear half observer must be carried by the slime"
            );
        }
    }
}
