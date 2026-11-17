//! Dispenser and dropper block behavior.
//!
//! Parity notes:
//! - Droppers move a whole stack into the container they face; dispensers eject items as
//!   item entities by default.
//! - A rising redstone edge schedules a two-game-tick delay before dispensing, matching
//!   Vanilla's `DispenserBlock`.
//! - `TRIGGERED` prevents a dispenser from firing repeatedly while a signal stays high.
//!
//! Not yet implemented: Vanilla's per-item dispense table (arrows, eggs, buckets,
//! bonemeal, TNT, minecarts, ...). Everything currently falls back to ejecting the item.

use std::sync::{Arc, Weak};

use glam::DVec3;
use steel_macros::block_behavior;
use steel_registry::blocks::BlockRef;
use steel_registry::blocks::block_state_ext::BlockStateExt as _;
use steel_registry::blocks::properties::{BlockStateProperties, BoolProperty, Direction, EnumProperty};
use steel_registry::item_stack::ItemStack;
use steel_registry::vanilla_block_entity_types;
use steel_registry::vanilla_entities;
use steel_utils::{BlockPos, BlockStateId, Downcast as _, types::UpdateFlags};

use crate::behavior::block::{BlockBehavior, BlockEntityCreation};
use crate::behavior::BlockPlaceContext;
use crate::block_entity::{BlockEntity as _, BLOCK_ENTITIES, entities::DispenserBlockEntity};
use crate::entity::{entities::ItemEntity, next_entity_id};
use crate::inventory::container::{Container, calculate_redstone_signal_from_containers};
use crate::inventory::lock::{ContainerLockGuard, ContainerRef};
use crate::inventory::transfer::insert_stack;
use crate::world::{SignalGetter as _, World};

const FACING: &EnumProperty<Direction> = &BlockStateProperties::FACING;
const TRIGGERED: &BoolProperty = &BlockStateProperties::TRIGGERED;

/// Vanilla dispenser block behavior.
#[block_behavior]
pub struct DispenserBlock {
    block: BlockRef,
}

/// Vanilla dropper block behavior.
#[block_behavior]
pub struct DropperBlock {
    block: BlockRef,
}

fn placement_state(block: BlockRef, context: &BlockPlaceContext<'_>) -> BlockStateId {
    block
        .default_state()
        .set_value(FACING, context.get_nearest_looking_direction().opposite())
        .set_value(TRIGGERED, false)
}

fn analog_output(world: &World, pos: BlockPos) -> i32 {
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

/// Ejects `stack` as an item entity in front of the dispenser.
fn eject_item(world: &Arc<World>, pos: BlockPos, facing: Direction, stack: ItemStack) {
    if stack.is_empty() {
        return;
    }
    let (dx, dy, dz) = facing.offset();
    let velocity = DVec3::new(f64::from(dx), f64::from(dy), f64::from(dz)) * 0.5
        + DVec3::new(0.0, 0.1, 0.0);
    let entity = Arc::new(ItemEntity::with_item_and_velocity(
        &vanilla_entities::ITEM,
        next_entity_id(),
        pos.relative(facing).get_center(),
        stack,
        velocity,
        Arc::downgrade(world),
    ));
    entity.set_default_pickup_delay();
    if let Err(error) = world.try_add_entity(entity) {
        log::warn!("failed to eject dispensed item: {error}");
    }
}

/// Removes the first non-empty stack and either transfers it (dropper) or ejects it.
fn dispense(state: BlockStateId, world: &Arc<World>, pos: BlockPos, dropper: bool) {
    let facing = state.get_value(FACING);
    let Some(block_entity) = world.get_block_entity(pos) else {
        return;
    };
    let Some(dispenser) = block_entity.downcast_ref::<DispenserBlockEntity>() else {
        return;
    };
    let Some(own) = dispenser.container_ref() else {
        return;
    };

    let target = if dropper {
        world
            .get_block_entity(pos.relative(facing))
            .and_then(|entity| entity.container_ref())
    } else {
        None
    };

    let mut containers: Vec<&ContainerRef> = vec![&own];
    if let Some(target) = &target {
        containers.push(target);
    }
    let Ok(mut guard) = ContainerLockGuard::lock_all(&containers) else {
        return;
    };

    let own_id = own.container_id();
    let Some(size) = guard.get(own_id).map(Container::get_container_size) else {
        return;
    };
    let mut picked: Option<(usize, ItemStack)> = None;
    for slot in 0..size {
        let Some(stack) = guard.get(own_id).map(|container| container.get_item(slot).clone()) else {
            return;
        };
        if !stack.is_empty() {
            picked = Some((slot, stack));
            break;
        }
    }
    let Some((slot, stack)) = picked else {
        return;
    };

    if let Some(target) = &target {
        let leftover = insert_stack(&mut guard, target.container_id(), stack.clone());
        if leftover.count() != stack.count() {
            if leftover.is_empty() {
                guard.remove_item(own_id, slot, stack.count());
            } else {
                guard.set_item(own_id, slot, leftover);
            }
            return;
        }
        // Nothing fit into the target container: fall through and eject the stack.
    }

    guard.remove_item(own_id, slot, stack.count());
    drop(guard);
    eject_item(world, pos, facing, stack);
}

macro_rules! impl_dispenser_block {
    ($name:ident, $dropper:expr, $new_entity:ident) => {
        impl $name {
            /// Creates this behavior for its registered block.
            #[must_use]
            pub const fn new(block: BlockRef) -> Self {
                Self { block }
            }
        }

        impl BlockBehavior for $name {
            fn get_state_for_placement(
                &self,
                context: &BlockPlaceContext<'_>,
            ) -> Option<BlockStateId> {
                Some(placement_state(self.block, context))
            }

            fn handle_neighbor_changed(
                &self,
                state: BlockStateId,
                world: &Arc<World>,
                pos: BlockPos,
                _source_block: BlockRef,
                _moved_by_piston: bool,
            ) {
                let powered = world.has_neighbor_signal(pos);
                let triggered = state.get_value(TRIGGERED);
                if powered && !triggered {
                    world.schedule_block_tick_default(pos, self.block, 2);
                } else if !powered && triggered {
                    world.set_block(
                        pos,
                        state.set_value(TRIGGERED, false),
                        UpdateFlags::UPDATE_CLIENTS,
                    );
                }
            }

            fn tick(&self, state: BlockStateId, world: &Arc<World>, pos: BlockPos) {
                if !world.has_neighbor_signal(pos) {
                    return;
                }
                if !state.get_value(TRIGGERED) {
                    world.set_block(
                        pos,
                        state.set_value(TRIGGERED, true),
                        UpdateFlags::UPDATE_CLIENTS,
                    );
                }
                dispense(state, world, pos, $dropper);
            }

            fn new_block_entity(
                &self,
                level: Weak<World>,
                pos: BlockPos,
                state: BlockStateId,
            ) -> BlockEntityCreation {
                BlockEntityCreation::from_registered_factory(BLOCK_ENTITIES.create(
                    &vanilla_block_entity_types::$new_entity,
                    level,
                    pos,
                    state,
                ))
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
                analog_output(world, pos)
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
        }
    };
}

impl_dispenser_block!(DispenserBlock, false, DISPENSER);
impl_dispenser_block!(DropperBlock, true, DROPPER);

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use steel_registry::{
        blocks::block_state_ext::BlockStateExt as _, blocks::properties::Direction,
        init_vanilla_registry, item_stack::ItemStack, vanilla_blocks, vanilla_items,
    };
    use steel_utils::{BlockPos, ChunkPos, Downcast as _, types::UpdateFlags};

    use super::{DispenserBlock, DropperBlock, FACING, TRIGGERED};
    use crate::behavior::{BlockBehavior, init_behaviors};
    use crate::block_entity::{
        entities::DispenserBlockEntity, init_block_entities,
    };
    use crate::inventory::lock::{ContainerLockGuard, ContainerRef};
    use crate::test_support::{TestWorld, dropped_items, fresh_test_world, insert_ready_full_chunk};
    use crate::world::World;

    const DISPENSER_POS: BlockPos = BlockPos::new(0, 0, 0);
    const POWER_POS: BlockPos = BlockPos::new(0, 1, 0);
    const BELOW_POS: BlockPos = BlockPos::new(0, -1, 0);

    fn fixture(name: &'static str) -> TestWorld {
        init_vanilla_registry();
        init_behaviors();
        init_block_entities();
        let fixture = fresh_test_world(name);
        insert_ready_full_chunk(&fixture.world, ChunkPos::new(0, 0));
        fixture
    }

    fn facing_down(block: steel_registry::blocks::BlockRef) -> steel_utils::BlockStateId {
        block
            .default_state()
            .set_value(FACING, Direction::Down)
            .set_value(TRIGGERED, false)
    }

    fn place_machine(world: &Arc<World>, state: steel_utils::BlockStateId, with_barrel_below: bool) {
        if with_barrel_below {
            assert!(world.set_block(
                BELOW_POS,
                vanilla_blocks::BARREL.default_state(),
                UpdateFlags::UPDATE_NONE
            ));
        }
        assert!(world.set_block(
            POWER_POS,
            vanilla_blocks::REDSTONE_BLOCK.default_state(),
            UpdateFlags::UPDATE_NONE
        ));
        assert!(world.set_block(DISPENSER_POS, state, UpdateFlags::UPDATE_NONE));
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

    #[test]
    fn placing_dispenser_registers_shared_block_entity() {
        let fixture = fixture("dispenser_be");
        let state = facing_down(&vanilla_blocks::DISPENSER);
        place_machine(&fixture.world, state, false);
        assert!(
            fixture
                .world
                .get_block_entity(DISPENSER_POS)
                .expect("entity")
                .downcast_ref::<DispenserBlockEntity>()
                .is_some()
        );
    }

    #[test]
    fn dropper_moves_whole_stack_into_container_below() {
        let fixture = fixture("dropper_transfer");
        let state = facing_down(&vanilla_blocks::DROPPER);
        place_machine(&fixture.world, state, true);
        set_slot(
            &fixture.world,
            DISPENSER_POS,
            0,
            ItemStack::with_count(&vanilla_items::STONE, 32),
        );

        DropperBlock::new(&vanilla_blocks::DROPPER).tick(state, &fixture.world, DISPENSER_POS);

        assert_eq!(count_at(&fixture.world, BELOW_POS, 0), 32);
        assert_eq!(count_at(&fixture.world, DISPENSER_POS, 0), 0);
    }

    #[test]
    fn dropper_ejects_when_no_container_is_in_front() {
        let fixture = fixture("dropper_eject");
        let state = facing_down(&vanilla_blocks::DROPPER);
        place_machine(&fixture.world, state, false);
        set_slot(
            &fixture.world,
            DISPENSER_POS,
            0,
            ItemStack::with_count(&vanilla_items::STONE, 8),
        );
        let before = dropped_items(&fixture.world, DISPENSER_POS.get_center()).len();

        DropperBlock::new(&vanilla_blocks::DROPPER).tick(state, &fixture.world, DISPENSER_POS);

        assert_eq!(count_at(&fixture.world, DISPENSER_POS, 0), 0);
        assert_eq!(
            dropped_items(&fixture.world, DISPENSER_POS.get_center()).len(),
            before + 1
        );
    }

    #[test]
    fn dispenser_ejects_items_once_per_rising_edge() {
        let fixture = fixture("dispenser_eject");
        let state = facing_down(&vanilla_blocks::DISPENSER);
        place_machine(&fixture.world, state, false);
        set_slot(
            &fixture.world,
            DISPENSER_POS,
            0,
            ItemStack::with_count(&vanilla_items::STONE, 1),
        );
        let behavior = DispenserBlock::new(&vanilla_blocks::DISPENSER);
        let before = dropped_items(&fixture.world, DISPENSER_POS.get_center()).len();

        behavior.tick(state, &fixture.world, DISPENSER_POS);
        let triggered_state = fixture.world.get_block_state(DISPENSER_POS);
        assert!(triggered_state.get_value(TRIGGERED));
        assert_eq!(
            dropped_items(&fixture.world, DISPENSER_POS.get_center()).len(),
            before + 1
        );

        // A second tick with the signal still high must not fire again.
        behavior.tick(triggered_state, &fixture.world, DISPENSER_POS);
        assert_eq!(
            dropped_items(&fixture.world, DISPENSER_POS.get_center()).len(),
            before + 1
        );
    }

    #[test]
    fn falling_signal_clears_triggered() {
        let fixture = fixture("dispenser_falling_edge");
        let state = facing_down(&vanilla_blocks::DISPENSER);
        place_machine(&fixture.world, state, false);
        let triggered = state.set_value(TRIGGERED, true);
        assert!(fixture.world.set_block(DISPENSER_POS, triggered, UpdateFlags::UPDATE_NONE));
        assert!(fixture.world.set_block(
            POWER_POS,
            vanilla_blocks::AIR.default_state(),
            UpdateFlags::UPDATE_NONE
        ));

        DispenserBlock::new(&vanilla_blocks::DISPENSER).handle_neighbor_changed(
            triggered,
            &fixture.world,
            DISPENSER_POS,
            &vanilla_blocks::REDSTONE_BLOCK,
            false,
        );

        assert!(!fixture.world.get_block_state(DISPENSER_POS).get_value(TRIGGERED));
    }

    #[test]
    fn comparator_reads_container_fullness() {
        let fixture = fixture("dispenser_comparator");
        let state = facing_down(&vanilla_blocks::DISPENSER);
        place_machine(&fixture.world, state, false);
        let behavior = DispenserBlock::new(&vanilla_blocks::DISPENSER);
        assert_eq!(
            behavior.get_analog_output_signal(
                state,
                &fixture.world,
                DISPENSER_POS,
                Direction::Down
            ),
            0
        );

        for slot in 0..3 {
            set_slot(
                &fixture.world,
                DISPENSER_POS,
                slot,
                ItemStack::with_count(&vanilla_items::STONE, 64),
            );
        }
        assert!(
            behavior.get_analog_output_signal(
                state,
                &fixture.world,
                DISPENSER_POS,
                Direction::Down
            ) > 0
        );
    }
}
