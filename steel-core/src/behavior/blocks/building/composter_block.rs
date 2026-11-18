//! Comparator-visible composter state.
//!
//! Composting interactions and hopper automation are independent mechanics;
//! comparators read the block-state fill level directly in vanilla.

use std::sync::Arc;

use steel_macros::block_behavior;
use steel_registry::item_stack::ItemStack;
use steel_registry::{vanilla_blocks, vanilla_items};
use steel_registry::blocks::{
    BlockRef,
    block_state_ext::BlockStateExt as _,
    properties::{BlockStateProperties, Direction, IntProperty},
};
use steel_utils::{BlockPos, BlockStateId};

use crate::{
    behavior::{BlockBehavior, BlockPlaceContext},
    entity::ai::path::PathComputationType,
    world::World,
};

/// Vanilla composter behavior required by analog signal readers.
#[block_behavior]
pub struct ComposterBlock {
    block: BlockRef,
}

const LEVEL_COMPOSTER: &IntProperty = &BlockStateProperties::LEVEL_COMPOSTER;

impl ComposterBlock {
    /// Creates composter behavior.
    #[must_use]
    pub const fn new(block: BlockRef) -> Self {
        Self { block }
    }
}

/// Returns whether `stack` can be composted by this composter.
///
/// Tree farms feed saplings/propagules into the composter, so the full
/// sapling family is accepted; the list is intentionally explicit instead of
/// depending on a generated item tag that is not loaded in headless tests.
pub fn is_compostable(stack: &ItemStack) -> bool {
    stack.is(&vanilla_items::OAK_SAPLING)
        || stack.is(&vanilla_items::SPRUCE_SAPLING)
        || stack.is(&vanilla_items::BIRCH_SAPLING)
        || stack.is(&vanilla_items::JUNGLE_SAPLING)
        || stack.is(&vanilla_items::ACACIA_SAPLING)
        || stack.is(&vanilla_items::DARK_OAK_SAPLING)
        || stack.is(&vanilla_items::CHERRY_SAPLING)
        || stack.is(&vanilla_items::MANGROVE_PROPAGULE)
}

/// Feeds one compostable item into the composter, advancing its fill level.
///
/// Returns `true` when one item was consumed. A full composter (level 8) is
/// ready to be harvested and refuses further input, matching vanilla's
/// "ready" state.
pub fn try_compost(world: &Arc<World>, pos: BlockPos, stack: &ItemStack) -> bool {
    if !is_compostable(stack) {
        return false;
    }
    let state = world.get_block_state(pos);
    if state.get_block() != &vanilla_blocks::COMPOSTER {
        return false;
    }
    let level = state.get_value(LEVEL_COMPOSTER);
    if level >= LEVEL_COMPOSTER.max {
        return false;
    }
    let next = state.set_value(LEVEL_COMPOSTER, level + 1);
    world.set_block(pos, next, steel_utils::types::UpdateFlags::UPDATE_ALL);
    true
}

/// Harvests one bone meal from a fully-filled composter and resets its level.
pub fn harvest_bone_meal(world: &Arc<World>, pos: BlockPos) -> Option<ItemStack> {
    let state = world.get_block_state(pos);
    if state.get_block() != &vanilla_blocks::COMPOSTER
        || state.get_value(LEVEL_COMPOSTER) < LEVEL_COMPOSTER.max
    {
        return None;
    }
    world.set_block(
        pos,
        state.set_value(LEVEL_COMPOSTER, 0),
        steel_utils::types::UpdateFlags::UPDATE_ALL,
    );
    Some(ItemStack::new(&vanilla_items::BONE_MEAL))
}

impl BlockBehavior for ComposterBlock {
    fn get_state_for_placement(&self, _context: &BlockPlaceContext<'_>) -> Option<BlockStateId> {
        Some(self.block.default_state())
    }

    fn has_analog_output_signal(&self, _state: BlockStateId) -> bool {
        true
    }

    fn get_analog_output_signal(
        &self,
        state: BlockStateId,
        _world: &World,
        _pos: BlockPos,
        _direction: Direction,
    ) -> i32 {
        i32::from(state.get_value(LEVEL_COMPOSTER))
    }

    fn is_pathfindable(
        &self,
        _state: BlockStateId,
        _computation_type: PathComputationType,
    ) -> bool {
        false
    }
}

#[cfg(test)]
mod tests {
    use steel_registry::{init_vanilla_registry, vanilla_blocks};

    use super::*;
    use crate::{
        behavior::{BLOCK_BEHAVIORS, init_behaviors},
        test_support::fresh_test_world,
    };

    #[test]
    fn registered_composter_outputs_its_full_state_level() {
        init_vanilla_registry();
        init_behaviors();
        let world = fresh_test_world("composter_analog_output");
        let state = vanilla_blocks::COMPOSTER
            .default_state()
            .set_value(LEVEL_COMPOSTER, LEVEL_COMPOSTER.max);
        let behavior = BLOCK_BEHAVIORS.get_behavior(state.get_block());

        assert!(behavior.has_analog_output_signal(state));
        assert_eq!(
            behavior.get_analog_output_signal(
                state,
                world.world.as_ref(),
                BlockPos::ZERO,
                Direction::North,
            ),
            i32::from(LEVEL_COMPOSTER.max),
        );
    }

    #[test]
    fn composter_reaches_ready_at_level_eight_and_harvests_bone_meal() {
        init_vanilla_registry();
        init_behaviors();
        let world = fresh_test_world("composter_harvest");
        crate::test_support::insert_ready_full_chunk(&world.world, steel_utils::ChunkPos::new(0, 0));
        let pos = BlockPos::new(8, 64, 8);
        world.world.set_block(
            pos,
            vanilla_blocks::COMPOSTER.default_state(),
            steel_utils::types::UpdateFlags::UPDATE_ALL,
        );
        let sapling = steel_registry::item_stack::ItemStack::new(&steel_registry::vanilla_items::OAK_SAPLING);
        for _ in 0..8 {
            assert!(super::try_compost(&world.world, pos, &sapling));
        }
        let state = world.world.get_block_state(pos);
        assert_eq!(state.get_value(LEVEL_COMPOSTER), LEVEL_COMPOSTER.max);
        assert!(!super::try_compost(&world.world, pos, &sapling));
        let bone_meal = super::harvest_bone_meal(&world.world, pos).expect("ready composter must yield bone meal");
        assert!(bone_meal.is(&steel_registry::vanilla_items::BONE_MEAL));
        assert_eq!(world.world.get_block_state(pos).get_value(LEVEL_COMPOSTER), 0);
    }
}
