use crate::world::game_event::GameEventContext;
use std::sync::Arc;
use steel_protocol::packets::game::SoundSource;
use steel_registry::blocks::block_state_ext::BlockStateExt;
use steel_registry::item_stack::ItemStack;
use steel_registry::level_events;
use steel_registry::{sound_events, vanilla_blocks, vanilla_entities, vanilla_game_events};
use steel_utils::BlockPos;
use steel_utils::BlockStateId;
use steel_utils::types::UpdateFlags;

use super::DispenseItemBehavior;
use crate::behavior::blocks::FireBlock;
use crate::behavior::blocks::container::dispenser_block::FACING;
use crate::entity::{Entity, entities::PrimedTntEntity, next_entity_id};
use crate::world::World;

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

        // Vanilla: flint and steel facing TNT primes that TNT directly.
        if target_state.get_block() == &vanilla_blocks::TNT {
            let tnt = Arc::new(PrimedTntEntity::primed(
                &vanilla_entities::TNT,
                next_entity_id(),
                target_pos.get_bottom_center(),
                world,
                None,
            ));
            let tnt_pos = tnt.position();
            if world.try_add_entity(tnt).is_ok() {
                world.play_sound_at(
                    &sound_events::ENTITY_TNT_PRIMED,
                    SoundSource::Blocks,
                    tnt_pos,
                    1.0,
                    1.0,
                    None,
                );
                world.game_event(
                    &vanilla_game_events::ENTITY_PLACE,
                    target_pos,
                    &GameEventContext::default(),
                );
                world.remove_block(target_pos, false);
                success = true;
            }
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
