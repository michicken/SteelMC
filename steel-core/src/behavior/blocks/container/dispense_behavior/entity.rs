//! Dispense behavior that spawns a real entity from an item.

use std::sync::Arc;

use steel_registry::blocks::block_state_ext::BlockStateExt;
use steel_registry::entity_type::EntityTypeRef;
use steel_registry::item_stack::ItemStack;
use steel_registry::{level_events, vanilla_entities};
use steel_utils::{BlockPos, BlockStateId};

use super::{DefaultDispenseBehavior, DispenseItemBehavior};
use crate::behavior::blocks::container::dispenser_block::FACING;
use crate::entity::{EntitySpawnPlacement, EntitySpawnReason, EntitySpawnRequest, spawn_entity};
use crate::world::World;

/// Spawns one entity of `entity_type` in front of the dispenser.
pub struct SpawnEntityDispenseBehavior {
    entity_type: EntityTypeRef,
}

impl SpawnEntityDispenseBehavior {
    /// Creates the behavior for one entity type.
    #[must_use]
    pub const fn new(entity_type: EntityTypeRef) -> Self {
        Self { entity_type }
    }
}

impl DispenseItemBehavior for SpawnEntityDispenseBehavior {
    fn dispense(
        &self,
        world: &Arc<World>,
        pos: BlockPos,
        state: BlockStateId,
        mut item: ItemStack,
    ) -> ItemStack {
        let facing = state.get_value(FACING);
        let target_pos = pos.relative(facing);
        let request = EntitySpawnRequest {
            entity_type: self.entity_type,
            placement: EntitySpawnPlacement::Block {
                pos: target_pos,
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
        item.split(1);
        item
    }
}

/// Armor stand is a plain object; its entity key is `armor_stand`.
pub fn armor_stand_behavior() -> SpawnEntityDispenseBehavior {
    SpawnEntityDispenseBehavior::new(&vanilla_entities::ARMOR_STAND)
}
