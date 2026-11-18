//! Dispenser brush behavior: brushes an armadillo in front and drops a scute.

use std::sync::Arc;

use steel_registry::blocks::block_state_ext::BlockStateExt;
use steel_registry::item_stack::ItemStack;
use steel_registry::vanilla_items;
use steel_utils::{BlockPos, BlockStateId, Downcast as _, WorldAabb};

use super::{DefaultDispenseBehavior, DispenseItemBehavior};
use crate::behavior::blocks::container::dispenser_block::FACING;
use crate::entity::entities::ArmadilloEntity;
use crate::entity::Entity;
use crate::world::World;

/// Brushes the armadillo directly in front of the dispenser.
pub struct BrushDispenseBehavior;

impl DispenseItemBehavior for BrushDispenseBehavior {
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
        let Some(entity) = world
            .get_entities_in_aabb(&aabb)
            .into_iter()
            .find(|entity| entity.downcast_ref::<ArmadilloEntity>().is_some())
        else {
            return DefaultDispenseBehavior.dispense(world, pos, state, item);
        };
        let brushed = entity
            .downcast_ref::<ArmadilloEntity>()
            .is_some_and(ArmadilloEntity::brush);
        if !brushed {
            return DefaultDispenseBehavior.dispense(world, pos, state, item);
        }
        let drop_pos = entity.position() + glam::DVec3::new(0.0, 0.5, 0.0);
        let _ = world.spawn_item(drop_pos, ItemStack::new(&vanilla_items::ARMADILLO_SCUTE));
        let mut brush = item;
        if brush.hurt_and_break(1, false) {
            ItemStack::empty()
        } else {
            brush
        }
    }
}
