use std::sync::Arc;
use steel_registry::blocks::BlockRef;
use steel_registry::blocks::block_state_ext::BlockStateExt;
use steel_registry::item_stack::ItemStack;
use steel_registry::level_events;
use steel_utils::BlockPos;
use steel_utils::BlockStateId;
use steel_utils::Direction;

use super::DispenseItemBehavior;
use crate::behavior::blocks::container::dispenser_block::FACING;
use crate::behavior::context::BlockPlaceContext;
use crate::behavior::items::BlockItem;
use crate::world::World;

pub struct ShulkerBoxDispenseBehavior {
    block: BlockRef,
}

impl ShulkerBoxDispenseBehavior {
    pub const fn new(block: BlockRef) -> Self {
        Self { block }
    }
}

impl DispenseItemBehavior for ShulkerBoxDispenseBehavior {
    fn dispense(
        &self,
        world: &Arc<World>,
        pos: BlockPos,
        state: BlockStateId,
        mut item: ItemStack,
    ) -> ItemStack {
        let facing = state.get_value(FACING);
        let target_pos = pos.relative(facing);
        let clicked_face = if world.get_block_state(target_pos.below()).is_air() {
            facing
        } else {
            Direction::Up
        };

        let context =
            BlockPlaceContext::directional(world, target_pos, facing, &mut item, clicked_face);
        let placed = BlockItem::new(self.block).place(context).consumes_action();

        if !placed {
            world.level_event(level_events::SOUND_DISPENSER_FAIL, pos, 0, None);
        }

        item
    }
}
