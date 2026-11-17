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
    use steel_registry::blocks::properties::BlockStateProperties;
    use steel_registry::item_stack::ItemStack;
    use steel_registry::vanilla_items;
    use steel_utils::{BlockPos, ChunkPos, Identifier};

    use crate::inventory::lock::{ContainerLockGuard, ContainerRef};
    use crate::test_support::{
        TestPlayerBuilder, dropped_items, fresh_test_world, insert_ready_full_chunk,
        load_litematic, paste_litematic, tick_test_world, LitematicBuilder,
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

    fn set_slot(
        world: &Arc<crate::world::World>,
        pos: BlockPos,
        slot: usize,
        stack: ItemStack,
    ) {
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
        assert_eq!(block_id(region.state_at(0, 0, 0)), Identifier::new_static("minecraft", "stone"));
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
        assert!(report.failed_positions.is_empty(), "{:?}", report.failed_positions);
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
        builder.set(
            1,
            1,
            1,
            "minecraft:dispenser",
            &[("facing", "east")],
        );
        // Platform placed after the dispenser, so its neighbour update triggers the dispenser.
        builder.set(1, 0, 1, "minecraft:stone", &[]);
        builder.write(&path).expect("write litematic fixture");

        let litematic = load_litematic(&path).expect("load litematic fixture");
        let test_world = fresh_test_world("machine_projection_dispenser");
        let world: Arc<_> = Arc::clone(&test_world.world);
        insert_ready_full_chunk(&world, ChunkPos::new(0, 0));

        let origin = BlockPos::new(2, 2, 2);
        let report = paste_litematic(&world, &litematic, origin);
        assert!(report.failed_positions.is_empty(), "{:?}", report.failed_positions);
        set_slot(
            &world,
            origin.offset(1, 1, 1),
            0,
            ItemStack::with_count(&vanilla_items::STONE, 1),
        );

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
        assert!(report.failed_positions.is_empty(), "{:?}", report.failed_positions);
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
        assert!(report.failed_positions.is_empty(), "{:?}", report.failed_positions);

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
        assert!(report.failed_positions.is_empty(), "{:?}", report.failed_positions);
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
        assert!(report.failed_positions.is_empty(), "{:?}", report.failed_positions);
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
        assert_eq!(count_at(&world, origin.offset(0, 2, 0), 0), 2, "furnace input must load");
        assert_eq!(count_at(&world, origin.offset(0, 2, 0), 1), 4, "furnace fuel must load");
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
        assert!(barrel > 0, "hopper must extract smelted ingots into the barrel");
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
        assert!(report.failed_positions.is_empty(), "{:?}", report.failed_positions);
        assert!(report.failed_tile_entities.is_empty(), "{:?}", report.failed_tile_entities);
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
        assert!(report.failed_positions.is_empty(), "{:?}", report.failed_positions);
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
        assert!(barrel > 0, "hopper must move imported chest items into the barrel");
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
        assert!(report.failed_positions.is_empty(), "{:?}", report.failed_positions);
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
        assert!(bottom > 0, "the barrel across the chunk boundary must receive items");
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
        assert!(report.failed_positions.is_empty(), "{:?}", report.failed_positions);
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
        assert!(report.failed_positions.is_empty(), "{:?}", report.failed_positions);

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
}
