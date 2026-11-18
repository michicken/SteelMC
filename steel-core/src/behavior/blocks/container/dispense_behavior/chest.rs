//! Dispenser chest behavior: equips a chest on a donkey/mule/llama in front.

use std::sync::Arc;

use steel_registry::blocks::block_state_ext::BlockStateExt;
use steel_registry::item_stack::ItemStack;
use steel_utils::{BlockPos, BlockStateId, Downcast as _, WorldAabb};

use super::{DefaultDispenseBehavior, DispenseItemBehavior};
use crate::behavior::blocks::container::dispenser_block::FACING;
use crate::entity::entities::{
    DonkeyEntity, LlamaEntity, MuleEntity, TraderLlamaEntity,
};
use crate::world::World;

/// Equips a chest onto a chested-horse entity directly in front.
pub struct ChestDispenseBehavior;

impl ChestDispenseBehavior {
    fn equip(entity: &dyn crate::entity::Entity) -> bool {
        if let Some(donkey) = entity.downcast_ref::<DonkeyEntity>() {
            return donkey.equip_chest();
        }
        if let Some(mule) = entity.downcast_ref::<MuleEntity>() {
            return mule.equip_chest();
        }
        if let Some(llama) = entity.downcast_ref::<LlamaEntity>() {
            return llama.equip_chest();
        }
        if let Some(llama) = entity.downcast_ref::<TraderLlamaEntity>() {
            return llama.equip_chest();
        }
        false
    }
}

impl DispenseItemBehavior for ChestDispenseBehavior {
    fn dispense(
        &self,
        world: &Arc<World>,
        pos: BlockPos,
        state: BlockStateId,
        item: ItemStack,
    ) -> ItemStack {
        let front = pos.relative(state.get_value(FACING));
        let aabb = WorldAabb::new(
            f64::from(front.x()),
            f64::from(front.y()),
            f64::from(front.z()),
            f64::from(front.x() + 1),
            f64::from(front.y() + 1),
            f64::from(front.z() + 1),
        );
        let entities = world.get_entities_in_aabb(&aabb);
        for entity in entities {
            if Self::equip(entity.as_ref()) {
                let mut chest = item;
                if chest.split(1).is_empty() {
                    return ItemStack::empty();
                }
                return chest;
            }
        }
        DefaultDispenseBehavior.dispense(world, pos, state, item)
    }
}
