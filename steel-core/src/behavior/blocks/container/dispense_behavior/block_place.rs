use std::sync::Arc;

use steel_registry::blocks::block_state_ext::BlockStateExt as _;
use steel_registry::blocks::BlockRef;
use steel_registry::item_stack::ItemStack;
use steel_registry::level_events;
use steel_utils::types::UpdateFlags;
use steel_utils::{BlockPos, BlockStateId};

use super::{DefaultDispenseBehavior, DispenseItemBehavior};
use crate::behavior::blocks::container::dispenser_block::FACING;
use crate::world::World;

/// Vanilla `DispenseItemBehavior` for items that place a block in front of the
/// dispenser (carved pumpkin, skulls, etc.).
pub struct BlockPlaceDispenseBehavior {
    block: BlockRef,
}

impl BlockPlaceDispenseBehavior {
    #[must_use]
    pub const fn new(block: BlockRef) -> Self {
        Self { block }
    }
}

impl DispenseItemBehavior for BlockPlaceDispenseBehavior {
    fn dispense(
        &self,
        world: &Arc<World>,
        pos: BlockPos,
        state: BlockStateId,
        item: ItemStack,
    ) -> ItemStack {
        let facing = state.get_value(FACING);
        let target_pos = pos.relative(facing);
        let target_state = world.get_block_state(target_pos);
        if !target_state.is_air() && !target_state.is_replaceable() {
            return DefaultDispenseBehavior.dispense(world, pos, state, item);
        }

        world.set_block(
            target_pos,
            self.block.default_state(),
            UpdateFlags::UPDATE_ALL,
        );
        world.level_event(level_events::SOUND_DISPENSER_DISPENSE, pos, 0, None);
        let mut remaining = item;
        remaining.split(1);
        remaining
    }
}
