//! Hopper block behavior.
//!
//! Parity notes:
//! - Default facing is down; horizontal facings follow the player's look direction.
//! - `enabled` mirrors Vanilla's `ENABLED` property: a hopper stops transferring while
//!   receiving a redstone signal, and keeps its contents.
//! - Comparator output is the container fullness signal of its five slots.

use std::sync::{Arc, Weak};

use steel_macros::block_behavior;
use steel_registry::block_entity_type::BlockEntityTypeRef;
use steel_registry::blocks::BlockRef;
use steel_registry::blocks::block_state_ext::BlockStateExt as _;
use steel_registry::blocks::properties::{BlockStateProperties, BoolProperty, Direction, EnumProperty};
use steel_registry::vanilla_block_entity_types;
use steel_utils::{BlockPos, BlockStateId, types::UpdateFlags};

use crate::behavior::block::{BlockBehavior, BlockEntityCreation};
use crate::entity::ai::path::PathComputationType;
use crate::behavior::BlockPlaceContext;
use crate::block_entity::{BLOCK_ENTITIES, BlockEntityTicker};
use crate::inventory::container::calculate_redstone_signal_from_containers;
use crate::inventory::lock::{ContainerLockGuard, ContainerRef};
use crate::world::{SignalGetter as _, World};

const FACING: &EnumProperty<Direction> = &BlockStateProperties::FACING_HOPPER;
const ENABLED: &BoolProperty = &BlockStateProperties::ENABLED;

/// Vanilla hopper block behavior.
#[block_behavior]
pub struct HopperBlock {
    block: BlockRef,
}

impl HopperBlock {
    /// Creates hopper behavior for its registered block.
    #[must_use]
    pub const fn new(block: BlockRef) -> Self {
        Self { block }
    }

    fn is_enabled(world: &World, pos: BlockPos) -> bool {
        !world.has_neighbor_signal(pos)
    }
}

impl BlockBehavior for HopperBlock {
    fn get_state_for_placement(&self, context: &BlockPlaceContext<'_>) -> Option<BlockStateId> {
        // Vanilla `FACING_HOPPER` excludes UP; a player looking straight down resolves
        // to UP after the inversion, so clamp it back to DOWN.
        let mut facing = context.get_nearest_looking_direction().opposite();
        if facing == Direction::Up {
            facing = Direction::Down;
        }
        Some(
            self.block
                .default_state()
                .set_value(FACING, facing)
                .set_value(ENABLED, Self::is_enabled(context.world, context.place_pos())),
        )
    }

    fn handle_neighbor_changed(
        &self,
        state: BlockStateId,
        world: &Arc<World>,
        pos: BlockPos,
        _source_block: BlockRef,
        _moved_by_piston: bool,
    ) {
        let enabled = Self::is_enabled(world, pos);
        if state.get_value(ENABLED) == enabled {
            return;
        }
        world.set_block(
            pos,
            state.set_value(ENABLED, enabled),
            UpdateFlags::UPDATE_CLIENTS,
        );
    }

    fn new_block_entity(
        &self,
        level: Weak<World>,
        pos: BlockPos,
        state: BlockStateId,
    ) -> BlockEntityCreation {
        BlockEntityCreation::from_registered_factory(BLOCK_ENTITIES.create(
            &vanilla_block_entity_types::HOPPER,
            level,
            pos,
            state,
        ))
    }

    fn get_block_entity_ticker(
        &self,
        _world: &Arc<World>,
        _state: BlockStateId,
        block_entity_type: BlockEntityTypeRef,
    ) -> Option<BlockEntityTicker> {
        BlockEntityTicker::for_matching_entity_tick(
            block_entity_type,
            &vanilla_block_entity_types::HOPPER,
        )
    }

    fn has_analog_output_signal(&self, _state: BlockStateId) -> bool {
        true
    }

    fn get_analog_output_signal(
        &self,
        _state: BlockStateId,
        world: &World,
        pos: BlockPos,
        _direction: Direction,
    ) -> i32 {
        let Some(container) = world
            .get_block_entity(pos)
            .and_then(ContainerRef::from_block_entity)
        else {
            return 0;
        };
        let Ok(guard) = ContainerLockGuard::lock_all(&[&container]) else {
            return 0;
        };
        calculate_redstone_signal_from_containers(guard.get(container.container_id()))
    }

    fn affect_neighbors_after_removal(
        &self,
        state: BlockStateId,
        world: &Arc<World>,
        pos: BlockPos,
        _moved: bool,
    ) {
        world.update_neighbor_for_output_signal(pos, state.get_block());
    }

    fn is_pathfindable(&self, _state: BlockStateId, _kind: PathComputationType) -> bool {
        false
    }
}

#[cfg(test)]
mod tests {
    use steel_registry::blocks::block_state_ext::BlockStateExt as _;
    use steel_registry::blocks::properties::Direction;
    use steel_registry::vanilla_blocks;

    use super::FACING;

    #[test]
    fn hopper_facing_property_excludes_up() {
        let values = FACING.possible_values;
        assert!(!values.contains(&Direction::Up));
        assert!(values.contains(&Direction::Down));
    }

    #[test]
    fn default_hopper_state_is_downwards_and_enabled() {
        steel_registry::init_vanilla_registry();
        let state = vanilla_blocks::HOPPER.default_state();
        assert_eq!(state.get_value(FACING), Direction::Down);
        assert!(state.get_value(super::ENABLED));
    }
}

#[cfg(test)]
mod integration_tests {
    use std::sync::Arc;

    use glam::DVec3;
    use steel_registry::{
        blocks::block_state_ext::BlockStateExt as _, init_vanilla_registry,
        item_stack::ItemStack, vanilla_blocks, vanilla_entities, vanilla_items,
    };
    use steel_utils::{BlockPos, ChunkPos, Downcast as _, types::UpdateFlags};

    use super::{ENABLED, FACING};
    use crate::behavior::init_behaviors;
    use crate::block_entity::{entities::HopperBlockEntity, init_block_entities};
    use crate::entity::{entities::ItemEntity, next_entity_id};
    use crate::inventory::lock::{ContainerLockGuard, ContainerRef};
    use crate::test_support::{TestWorld, fresh_test_world, insert_ready_full_chunk};
    use crate::world::World;

    const HOPPER_POS: BlockPos = BlockPos::new(0, 0, 0);
    const ABOVE_POS: BlockPos = BlockPos::new(0, 1, 0);
    const BELOW_POS: BlockPos = BlockPos::new(0, -1, 0);

    fn fixture(name: &'static str) -> TestWorld {
        init_vanilla_registry();
        init_behaviors();
        init_block_entities();
        let fixture = fresh_test_world(name);
        insert_ready_full_chunk(&fixture.world, ChunkPos::new(0, 0));
        fixture
    }

    fn container_at(world: &Arc<World>, pos: BlockPos) -> ContainerRef {
        ContainerRef::from_block_entity(world.get_block_entity(pos).expect("block entity"))
            .expect("container")
    }

    fn count_at(world: &Arc<World>, pos: BlockPos, slot: usize) -> i32 {
        let container = container_at(world, pos);
        let guard = ContainerLockGuard::lock_all(&[&container]).expect("lock");
        guard
            .get(container.container_id())
            .expect("container")
            .get_item(slot)
            .count()
    }

    fn set_slot(world: &Arc<World>, pos: BlockPos, slot: usize, stack: ItemStack) {
        let container = container_at(world, pos);
        let mut guard = ContainerLockGuard::lock_all(&[&container]).expect("lock");
        guard.set_item(container.container_id(), slot, stack);
    }

    fn tick_hopper(world: &Arc<World>, ticks: usize) {
        let block_entity = world.get_block_entity(HOPPER_POS).expect("hopper entity");
        for _ in 0..ticks {
            block_entity.tick(world);
        }
    }

    fn place_machine(world: &Arc<World>, hopper_state: steel_utils::BlockStateId) {
        assert!(world.set_block(BELOW_POS, vanilla_blocks::HOPPER.default_state(), UpdateFlags::UPDATE_NONE));
        assert!(world.set_block(ABOVE_POS, vanilla_blocks::BARREL.default_state(), UpdateFlags::UPDATE_NONE));
        assert!(world.set_block(HOPPER_POS, hopper_state, UpdateFlags::UPDATE_NONE));
    }

    #[test]
    fn placing_hopper_registers_the_block_entity() {
        let fixture = fixture("hopper_be");
        place_machine(&fixture.world, vanilla_blocks::HOPPER.default_state());
        let block_entity = fixture.world.get_block_entity(HOPPER_POS).expect("entity");
        assert!(block_entity.downcast_ref::<HopperBlockEntity>().is_some());
    }

    #[test]
    fn hopper_pulls_from_above_and_pushes_below() {
        let fixture = fixture("hopper_transport");
        place_machine(&fixture.world, vanilla_blocks::HOPPER.default_state());
        set_slot(
            &fixture.world,
            ABOVE_POS,
            0,
            ItemStack::with_count(&vanilla_items::STONE, 32),
        );

        // One activation pulls exactly one item, then the hopper cools down for 7 more ticks.
        tick_hopper(&fixture.world, 8);
        assert_eq!(count_at(&fixture.world, ABOVE_POS, 0), 31);
        assert_eq!(count_at(&fixture.world, HOPPER_POS, 0), 1);
        assert_eq!(count_at(&fixture.world, BELOW_POS, 0), 0);

        // The next activation first pushes its buffered item down, then pulls a new one in,
        // mirroring Vanilla's push-then-pull order.
        tick_hopper(&fixture.world, 8);
        assert_eq!(count_at(&fixture.world, ABOVE_POS, 0), 30);
        assert_eq!(count_at(&fixture.world, HOPPER_POS, 0), 1);
        assert_eq!(count_at(&fixture.world, BELOW_POS, 0), 1);
    }

    #[test]
    fn disabled_hopper_keeps_items_in_place() {
        let fixture = fixture("hopper_disabled");
        let disabled = vanilla_blocks::HOPPER
            .default_state()
            .set_value(ENABLED, false);
        place_machine(&fixture.world, disabled);
        set_slot(
            &fixture.world,
            ABOVE_POS,
            0,
            ItemStack::with_count(&vanilla_items::STONE, 5),
        );

        tick_hopper(&fixture.world, 8);
        assert_eq!(count_at(&fixture.world, ABOVE_POS, 0), 5);
        assert_eq!(count_at(&fixture.world, HOPPER_POS, 0), 0);
    }

    #[test]
    fn comparator_reads_hopper_fullness() {
        use crate::behavior::BLOCK_BEHAVIORS;
        use steel_registry::blocks::properties::Direction;

        let fixture = fixture("hopper_comparator");
        place_machine(&fixture.world, vanilla_blocks::HOPPER.default_state());

        let state = fixture.world.get_block_state(HOPPER_POS);
        let behavior = BLOCK_BEHAVIORS.get_behavior(state.get_block());
        assert_eq!(
            behavior.get_analog_output_signal(state, &fixture.world, HOPPER_POS, Direction::Down),
            0
        );

        set_slot(
            &fixture.world,
            HOPPER_POS,
            0,
            ItemStack::with_count(&vanilla_items::STONE, 64),
        );
        assert!(
            behavior.get_analog_output_signal(state, &fixture.world, HOPPER_POS, Direction::Down)
                > 0
        );
    }

    #[test]
    fn hopper_sucks_item_entities_from_above() {
        let fixture = fixture("hopper_suck");
        assert!(fixture.world.set_block(
            HOPPER_POS,
            vanilla_blocks::HOPPER.default_state(),
            UpdateFlags::UPDATE_NONE
        ));

        let item = Arc::new(ItemEntity::with_item(
            &vanilla_entities::ITEM,
            next_entity_id(),
            DVec3::new(0.5, 1.2, 0.5),
            ItemStack::with_count(&vanilla_items::STONE, 3),
            Arc::downgrade(&fixture.world),
        ));
        fixture.world.try_add_entity(item).expect("spawn item entity");

        tick_hopper(&fixture.world, 1);
        assert_eq!(count_at(&fixture.world, HOPPER_POS, 0), 3);
    }

    #[test]
    fn facing_property_round_trips_through_placement_state() {
        let fixture = fixture("hopper_facing");
        let north = vanilla_blocks::HOPPER
            .default_state()
            .set_value(FACING, steel_registry::blocks::properties::Direction::North);
        place_machine(&fixture.world, north);
        assert_eq!(
            fixture.world.get_block_state(HOPPER_POS).get_value(FACING),
            steel_registry::blocks::properties::Direction::North
        );
    }
}
