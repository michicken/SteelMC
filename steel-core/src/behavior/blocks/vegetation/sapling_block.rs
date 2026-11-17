use std::sync::Arc;

use rand::{Rng, RngExt};
use steel_macros::block_behavior;
use steel_registry::blocks::block_state_ext::BlockStateExt as _;
use steel_registry::blocks::properties::BlockStateProperties;
use steel_registry::feature::{ConfiguredFeature, ConfiguredFeatureKind};
use steel_registry::vanilla_block_tags::BlockTag;
use steel_registry::{REGISTRY, vanilla_blocks, vanilla_configured_features};
use steel_utils::random::worldgen_random::WorldgenRandom;
use steel_utils::types::UpdateFlags;
use steel_utils::{BlockPos, BlockStateId};
use steel_worldgen::biomes::obfuscate_biome_seed;

use crate::behavior::block::BlockBehavior;
use crate::behavior::blocks::vegetation::bonemealable::Bonemealable;
use crate::behavior::context::BlockPlaceContext;
use crate::world::{LevelAccessor, LevelReader, ScheduledTickAccess, World};
use crate::worldgen::feature::FeatureDecorationRunner;
use crate::worldgen::feature::no_nested_features;

use super::{BlockRef, default_surviving_state, survives_on_tag};

/// Vanilla `SaplingBlock` survival.
// Vanilla saplings grow into their configured tree feature on random tick and
// when bonemealed; both paths deliberately go through the same live-world
// feature runner used by mangrove propagules.
#[block_behavior]
pub struct SaplingBlock {
    block: BlockRef,
}

impl SaplingBlock {
    /// Creates a new sapling block behavior.
    #[must_use]
    pub const fn new(block: BlockRef) -> Self {
        Self { block }
    }
}

impl SaplingBlock {

    fn grow_tree(
        world: &Arc<World>,
        pos: BlockPos,
        state: BlockStateId,
        rng: &mut dyn Rng,
    ) -> bool {
        let feature = Self::tree_feature_for_state(state);
        let ConfiguredFeatureKind::Tree(config) = &feature.kind else {
            return false;
        };
        world.set_block(
            pos,
            vanilla_blocks::AIR.default_state(),
            UpdateFlags::UPDATE_NONE,
        );

        let mut worldgen_random = WorldgenRandom::from_seed(rng.random());
        let mut level = Arc::clone(world);
        let placed = FeatureDecorationRunner::place_tree_feature(
            &mut level,
            &REGISTRY,
            &mut worldgen_random,
            config,
            pos,
            obfuscate_biome_seed(world.seed()),
            no_nested_features,
        );

        if placed {
            if world.get_block_state(pos).is_air() {
                world.send_block_updated(pos);
            }
        } else {
            world.set_block(pos, state, UpdateFlags::UPDATE_NONE);
        }
        placed
    }

    fn tree_feature_for_state(state: BlockStateId) -> &'static ConfiguredFeature {
        let block = state.get_block();
        if block == &vanilla_blocks::OAK_SAPLING {
            &vanilla_configured_features::OAK
        } else if block == &vanilla_blocks::BIRCH_SAPLING {
            &vanilla_configured_features::BIRCH
        } else if block == &vanilla_blocks::SPRUCE_SAPLING {
            &vanilla_configured_features::SPRUCE
        } else if block == &vanilla_blocks::JUNGLE_SAPLING {
            &vanilla_configured_features::JUNGLE_TREE
        } else if block == &vanilla_blocks::ACACIA_SAPLING {
            &vanilla_configured_features::ACACIA
        } else if block == &vanilla_blocks::DARK_OAK_SAPLING {
            &vanilla_configured_features::DARK_OAK
        } else if block == &vanilla_blocks::CHERRY_SAPLING {
            &vanilla_configured_features::CHERRY
        } else if block == &vanilla_blocks::PALE_OAK_SAPLING {
            &vanilla_configured_features::PALE_OAK
        } else {
            &vanilla_configured_features::OAK
        }
    }
}

impl Bonemealable for SaplingBlock {
    fn is_valid_bonemeal_target(
        &self,
        _state: BlockStateId,
        _world: &dyn LevelReader,
        _pos: BlockPos,
    ) -> bool {
        true
    }

    fn perform_bonemeal(
        &self,
        state: BlockStateId,
        world: &Arc<World>,
        rng: &mut dyn Rng,
        pos: BlockPos,
    ) {
        Self::grow_tree(world, pos, state, rng);
    }
}

impl BlockBehavior for SaplingBlock {
    fn as_bonemealable(&self) -> Option<&dyn Bonemealable> {
        Some(self)
    }

    fn random_tick(&self, state: BlockStateId, world: &Arc<World>, pos: BlockPos) {
        let mut rng = rand::rng();
        if state.get_value(&BlockStateProperties::STAGE) == 0 {
            world.set_block(
                pos,
                state.set_value(&BlockStateProperties::STAGE, 1),
                UpdateFlags::UPDATE_CLIENTS,
            );
        } else if rng.random_range(0..7) == 0 {
            Self::grow_tree(world, pos, state, &mut rng);
        }
    }

    fn can_survive(&self, _state: BlockStateId, world: &dyn LevelReader, pos: BlockPos) -> bool {
        survives_on_tag(world, pos, &BlockTag::SUPPORTS_VEGETATION)
    }

    fn get_state_for_placement(&self, context: &BlockPlaceContext<'_>) -> Option<BlockStateId> {
        default_surviving_state(self.block, self, context)
    }
}
