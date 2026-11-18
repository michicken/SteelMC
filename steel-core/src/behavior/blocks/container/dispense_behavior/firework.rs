use std::sync::Arc;

use glam::DVec3;
use steel_registry::blocks::block_state_ext::BlockStateExt as _;
use steel_registry::item_stack::ItemStack;
use steel_registry::level_events;
use steel_registry::vanilla_entities;
use steel_utils::{BlockPos, BlockStateId};

use super::{DefaultDispenseBehavior, DispenseItemBehavior};
use crate::behavior::blocks::container::dispenser_block::FACING;
use crate::entity::{Entity, entities::FireworkRocketEntity, next_entity_id};
use crate::world::World;

/// Vanilla firework-rocket dispense behavior: spawn a *launched* rocket (not
/// the bare entity factory default), give it the dispensed stack so its
/// fireworks component drives flight duration, then shoot it forward.
pub struct FireworkRocketDispenseBehavior;

impl DispenseItemBehavior for FireworkRocketDispenseBehavior {
    fn dispense(
        &self,
        world: &Arc<World>,
        pos: BlockPos,
        state: BlockStateId,
        item: ItemStack,
    ) -> ItemStack {
        let facing = state.get_value(FACING);
        let offset = facing.offset_vec();
        let spawn_pos = DVec3::new(
            f64::from(pos.x()) + 0.5 + f64::from(offset.x) * 0.7,
            f64::from(pos.y()) + 0.5 + f64::from(offset.y) * 0.7,
            f64::from(pos.z()) + 0.5 + f64::from(offset.z) * 0.7,
        );
        let rocket = Arc::new(FireworkRocketEntity::launched(
            &vanilla_entities::FIREWORK_ROCKET,
            next_entity_id(),
            spawn_pos,
            Arc::downgrade(world),
            item.clone(),
        ));
        let direction = DVec3::new(
            f64::from(offset.x),
            f64::from(offset.y),
            f64::from(offset.z),
        )
        .normalize();
        if let Some(projectile) = rocket.as_projectile() {
            projectile.shoot(direction, 0.5, 0.0);
        }
        if world.try_add_entity(rocket).is_err() {
            return DefaultDispenseBehavior.dispense(world, pos, state, item);
        }
        world.level_event(level_events::SOUND_DISPENSER_PROJECTILE_LAUNCH, pos, 0, None);
        let mut remaining = item;
        remaining.split(1);
        remaining
    }
}
