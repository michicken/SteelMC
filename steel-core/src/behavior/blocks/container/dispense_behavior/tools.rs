use crate::world::game_event::GameEventContext;
use std::sync::Arc;
use steel_registry::blocks::block_state_ext::BlockStateExt;
use steel_registry::blocks::properties::BlockStateProperties;
use steel_registry::item_stack::ItemStack;
use steel_registry::level_events;
use steel_registry::vanilla_game_events;
use steel_utils::BlockPos;
use steel_utils::BlockStateId;
use steel_utils::types::UpdateFlags;

use super::DispenseItemBehavior;
use crate::behavior::blocks::FireBlock;
use crate::behavior::blocks::container::dispenser_block::FACING;
use crate::world::World;

fn can_light(state: BlockStateId) -> bool {
    !state
        .try_get_value(&BlockStateProperties::LIT)
        .unwrap_or(true)
}

pub struct FlintAndSteelDispenseBehavior;

impl DispenseItemBehavior for FlintAndSteelDispenseBehavior {
    fn dispense(
        &self,
        world: &Arc<World>,
        pos: BlockPos,
        state: BlockStateId,
        mut item: ItemStack,
    ) -> ItemStack {
        let facing = state.get_value(FACING);
        let target_pos = pos.relative(facing);
        let target_state = world.get_block_state(target_pos);
        let mut success = false;

        if can_light(target_state) {
            world.set_block(
                target_pos,
                target_state.set_value(&BlockStateProperties::LIT, true),
                UpdateFlags::UPDATE_ALL_IMMEDIATE,
            );
            world.game_event(
                &vanilla_game_events::BLOCK_CHANGE,
                target_pos,
                &GameEventContext::new(None, None),
            );
            success = true;
        } else if FireBlock::can_be_placed_at(world, target_pos, facing) {
            world.set_block(
                target_pos,
                FireBlock::get_state(world.as_ref(), target_pos),
                UpdateFlags::UPDATE_ALL,
            );
            world.game_event(
                &vanilla_game_events::BLOCK_PLACE,
                target_pos,
                &GameEventContext::new(None, None),
            );
            success = true;
        }

        if success {
            item.hurt_and_break(1, false);
        } else {
            world.level_event(level_events::SOUND_DISPENSER_FAIL, pos, 0, None);
        }

        item
    }
}

// TODO: ShearsDispenseBehavior — needs beehive honey-level tracking
// (beehive_block.rs has none yet) and a generic Shearable dispatch covering
// sheep/mooshroom/snow golem/bogged, which doesn't exist. Only Sheep has its
// own shear() today; that alone would silently diverge from vanilla.

// TODO: BrushDispenseBehavior — needs the Armadillo entity, which doesn't exist.

/// Vanilla `ShearsDispenseItemBehavior`: shear a growing plant tip in front.
pub struct ShearsDispenseBehavior;

impl DispenseItemBehavior for ShearsDispenseBehavior {
    fn dispense(
        &self,
        world: &Arc<World>,
        pos: BlockPos,
        state: BlockStateId,
        mut item: ItemStack,
    ) -> ItemStack {
        let facing = state.get_value(FACING);
        let target_pos = pos.relative(facing);
        let target_state = world.get_block_state(target_pos);
        let behavior = crate::behavior::BLOCK_BEHAVIORS.get_behavior(target_state.get_block());
        let Some(plant) = behavior.as_growing_plant_head() else {
            world.level_event(level_events::SOUND_DISPENSER_FAIL, pos, 0, None);
            return item;
        };
        if plant.is_max_age(target_state) {
            world.level_event(level_events::SOUND_DISPENSER_FAIL, pos, 0, None);
            return item;
        }

        world.set_block(
            target_pos,
            plant.get_max_age_state(target_state),
            UpdateFlags::UPDATE_ALL,
        );
        world.play_block_sound(&steel_registry::sound_events::BLOCK_GROWING_PLANT_CROP, target_pos, 1.0, 1.0, None);
        item.hurt_and_break(1, false);
        world.level_event(level_events::SOUND_DISPENSER_DISPENSE, pos, 0, None);
        item
    }
}
