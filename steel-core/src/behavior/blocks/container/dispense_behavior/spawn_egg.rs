use std::sync::Arc;

use steel_registry::blocks::block_state_ext::BlockStateExt as _;
use steel_registry::data_components::components::EntityData;
use steel_registry::data_components::vanilla_components::ENTITY_DATA;
use steel_registry::item_stack::ItemStack;
use steel_registry::level_events;
use steel_utils::{BlockPos, BlockStateId};

use super::{DefaultDispenseBehavior, DispenseItemBehavior};
use crate::behavior::blocks::container::dispenser_block::FACING;
use crate::entity::{
    EntitySpawnPlacement, EntitySpawnReason, EntitySpawnRequest, spawn_entity,
};
use crate::world::World;

/// Vanilla data-driven spawn-egg dispense behavior. The entity type comes from
/// the stack's `minecraft:entity_data` component, not from the item key.
pub struct SpawnEggDispenseBehavior;

impl DispenseItemBehavior for SpawnEggDispenseBehavior {
    fn dispense(
        &self,
        world: &Arc<World>,
        pos: BlockPos,
        state: BlockStateId,
        item: ItemStack,
    ) -> ItemStack {
        let Some(entity_data) = item.get(ENTITY_DATA) else {
            return DefaultDispenseBehavior.dispense(world, pos, state, item);
        };
        let facing = state.get_value(FACING);
        let target_pos = pos.relative(facing);
        let target_state = world.get_block_state(target_pos);
        let spawn_pos = if target_state.is_air() || target_state.is_replaceable() {
            target_pos
        } else {
            pos.relative(facing)
        };
        let request = EntitySpawnRequest {
            entity_type: EntityData::entity_type(entity_data),
            placement: EntitySpawnPlacement::Block {
                pos: spawn_pos,
                try_move_down: false,
                moved_up: false,
            },
            reason: EntitySpawnReason::Dispenser,
            finalize_spawn: true,
            play_ambient_sound: false,
            item_stack: Some(&item),
            user_is_operator: false,
        };
        if spawn_entity(world, request).is_err() {
            return DefaultDispenseBehavior.dispense(world, pos, state, item);
        }
        world.level_event(level_events::SOUND_DISPENSER_DISPENSE, pos, 0, None);
        let mut remaining = item;
        remaining.split(1);
        remaining
    }
}
