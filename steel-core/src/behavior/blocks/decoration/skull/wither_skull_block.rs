use crate::behavior::blocks::SkullBlock;
use crate::behavior::blocks::decoration::skull::abstract_skull_block::AbstractSkullBlock;
use crate::behavior::{BlockBehavior, BlockEntityCreation, BlockLootContext, BlockPlaceContext};
use crate::block_entity::SharedBlockEntity;
use crate::entity::ai::path::PathComputationType;
use crate::entity::entities::WitherBossEntity;
use crate::entity::{EntitySpawnReason, SharedEntity, next_entity_id};
use crate::world::World;
use std::sync::{Arc, Weak};
use glam::DVec3;
use steel_macros::block_behavior;
use steel_registry::blocks::BlockRef;
use steel_registry::blocks::block_state_ext::BlockStateExt as _;
use steel_registry::item_stack::ItemStack;
use steel_registry::{vanilla_blocks, vanilla_entities};
use steel_utils::types::UpdateFlags;
use steel_utils::{BlockPos, BlockStateId};

/// Behavior for wither skull blocks.
#[block_behavior]
pub struct WitherSkullBlock {
    base: SkullBlock,
}

impl WitherSkullBlock {
    /// Creates a new wither skull block behavior for the given block.
    pub const fn new(block: BlockRef) -> Self {
        Self {
            base: SkullBlock::new(block),
        }
    }

    /// Vanilla `WitherSkullBlock.canSpawnMob`: three floor skulls on soul sand
    /// in one horizontal line, with room for the wither above them.
    fn try_spawn_wither(world: &Arc<World>, pos: BlockPos) -> bool {
        if world.get_block_state(pos).get_block() != &vanilla_blocks::WITHER_SKELETON_SKULL {
            return false;
        }
        if world.get_block_state(pos.below()).get_block() != &vanilla_blocks::SOUL_SAND {
            return false;
        }

        for direction in [(1, 0), (0, 1)] {
            let center = Self::find_line_center(world, pos, direction);
            let Some(center) = center else {
                continue;
            };
            if !Self::has_wither_room(world, center) {
                continue;
            }
            Self::spawn_wither(world, center);
            return true;
        }
        false
    }

    fn find_line_center(world: &Arc<World>, pos: BlockPos, direction: (i32, i32)) -> Option<BlockPos> {
        for offset in -1..=1 {
            let center = BlockPos::new(
                pos.x() + direction.0 * offset,
                pos.y(),
                pos.z() + direction.1 * offset,
            );
            let mut valid = true;
            for step in -1..=1 {
                let skull_pos = BlockPos::new(
                    center.x() + direction.0 * step,
                    center.y(),
                    center.z() + direction.1 * step,
                );
                if world.get_block_state(skull_pos).get_block()
                    != &vanilla_blocks::WITHER_SKELETON_SKULL
                    || world.get_block_state(skull_pos.below()).get_block()
                        != &vanilla_blocks::SOUL_SAND
                {
                    valid = false;
                    break;
                }
            }
            if valid {
                return Some(center);
            }
        }
        None
    }

    fn has_wither_room(world: &Arc<World>, center: BlockPos) -> bool {
        for dy in 1..=3 {
            for dx in -1..=1 {
                for dz in -1..=1 {
                    if !world
                        .get_block_state(BlockPos::new(center.x() + dx, center.y() + dy, center.z() + dz))
                        .is_air()
                    {
                        return false;
                    }
                }
            }
        }
        true
    }

    fn spawn_wither(world: &Arc<World>, center: BlockPos) {
        // Vanilla consumes the three skulls.
        for step in -1..=1 {
            let skull_pos = BlockPos::new(center.x() + step, center.y(), center.z());
            world.set_block(skull_pos, vanilla_blocks::AIR.default_state(), UpdateFlags::UPDATE_ALL);
        }

        let wither = WitherBossEntity::new(
            &vanilla_entities::WITHER,
            next_entity_id(),
            DVec3::new(
                f64::from(center.x()) + 0.5,
                f64::from(center.y()),
                f64::from(center.z()) + 0.5,
            ),
            Arc::downgrade(world),
        );
        let entity: SharedEntity = Arc::new(wither);
        if let Err(error) = world.try_add_entity(entity) {
            log::debug!("failed to spawn wither: {error}");
        }
        let _ = EntitySpawnReason::Triggered;
    }
}

impl BlockBehavior for WitherSkullBlock {
    fn on_place(
        &self,
        state: BlockStateId,
        world: &Arc<World>,
        pos: BlockPos,
        old_state: BlockStateId,
        moved_by_piston: bool,
    ) {
        self.base.on_place(state, world, pos, old_state, moved_by_piston);
        if !moved_by_piston {
            Self::try_spawn_wither(world, pos);
        }
    }

    fn get_state_for_placement(&self, context: &BlockPlaceContext<'_>) -> Option<BlockStateId> {
        self.base.get_state_for_placement(context)
    }

    fn handle_neighbor_changed(
        &self,
        state: BlockStateId,
        world: &Arc<World>,
        pos: BlockPos,
        source_block: BlockRef,
        moved_by_piston: bool,
    ) {
        self.base
            .handle_neighbor_changed(state, world, pos, source_block, moved_by_piston);
    }

    fn is_pathfindable(&self, state: BlockStateId, computation_type: PathComputationType) -> bool {
        self.base.is_pathfindable(state, computation_type)
    }

    fn new_block_entity(
        &self,
        level: Weak<World>,
        pos: BlockPos,
        state: BlockStateId,
    ) -> BlockEntityCreation {
        self.base.new_block_entity(level, pos, state)
    }

    fn get_clone_item_stack(
        &self,
        block: BlockRef,
        state: BlockStateId,
        block_entity: Option<SharedBlockEntity>,
        include_data: bool,
    ) -> Option<ItemStack> {
        self.base
            .get_clone_item_stack(block, state, block_entity, include_data)
    }

    fn get_drops(
        &self,
        state: BlockStateId,
        context: &BlockLootContext<'_>,
    ) -> Option<Vec<ItemStack>> {
        self.base.get_drops(state, context)
    }
}

impl AbstractSkullBlock for WitherSkullBlock {
    fn state_for_placement(&self, context: &BlockPlaceContext<'_>) -> Option<BlockStateId> {
        self.base.state_for_placement(context)
    }
}
