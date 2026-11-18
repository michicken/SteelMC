use std::sync::Arc;
use steel_registry::blocks::block_state_ext::BlockStateExt;
use steel_registry::item_stack::ItemStack;
use steel_registry::level_events;
use steel_utils::BlockPos;
use steel_utils::BlockStateId;

use super::DispenseItemBehavior;
use crate::behavior::blocks::container::dispenser_block::FACING;
use crate::world::World;

pub struct DefaultDispenseBehavior;

impl DispenseItemBehavior for DefaultDispenseBehavior {
    fn dispense(
        &self,
        world: &Arc<World>,
        pos: BlockPos,
        state: BlockStateId,
        mut item: ItemStack,
    ) -> ItemStack {
        let facing = state.get_value(FACING);
        let target_pos = pos.relative(facing);

        // Vanilla ejects exactly one item per activation; the rest stays in the slot.
        let ejected = item.split(1);
        world.drop_item_stack(target_pos, ejected);
        world.level_event(level_events::SOUND_DISPENSER_DISPENSE, pos, 0, None);
        item
    }
}
