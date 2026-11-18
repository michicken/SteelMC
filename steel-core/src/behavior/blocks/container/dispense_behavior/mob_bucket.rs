//! Dispenses a bucket mob and leaves an empty bucket behind.

use std::sync::Arc;

use steel_registry::blocks::block_state_ext::BlockStateExt;
use steel_registry::item_stack::ItemStack;
use steel_registry::{REGISTRY, RegistryExt as _, level_events, vanilla_items};
use steel_utils::{BlockPos, BlockStateId, Identifier};

use super::{DefaultDispenseBehavior, DispenseItemBehavior};
use crate::behavior::blocks::container::dispenser_block::FACING;
use crate::entity::{EntitySpawnPlacement, EntitySpawnReason, EntitySpawnRequest, spawn_entity};
use crate::world::World;

/// Drops the captured mob into the world and returns the empty bucket.
pub struct MobBucketDispenseBehavior;

impl DispenseItemBehavior for MobBucketDispenseBehavior {
    fn dispense(
        &self,
        world: &Arc<World>,
        pos: BlockPos,
        state: BlockStateId,
        item: ItemStack,
    ) -> ItemStack {
        let path = item.item().key.path.as_ref();
        let Some(entity_path) = path.strip_suffix("_bucket") else {
            return DefaultDispenseBehavior.dispense(world, pos, state, item);
        };
        let entity_key = Identifier::new_static("minecraft", entity_path);
        let Some(entity_type) = REGISTRY.entity_types.by_key(&entity_key) else {
            return DefaultDispenseBehavior.dispense(world, pos, state, item);
        };
        let facing = state.get_value(FACING);
        let target_pos = pos.relative(facing);
        let request = EntitySpawnRequest {
            entity_type,
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
        let mut remaining = item;
        remaining.split(1);
        if remaining.is_empty() {
            ItemStack::new(&vanilla_items::BUCKET)
        } else {
            remaining
        }
    }
}
