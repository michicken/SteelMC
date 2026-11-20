//! Copper chest behavior.

use std::sync::{Arc, Weak};

use steel_macros::block_behavior;
use steel_registry::blocks::block_state_ext::BlockStateExt as _;
use steel_registry::blocks::properties::{BlockStateProperties, ChestType};
use steel_registry::sound_event::SoundEventRef;
use steel_registry::vanilla_block_tags::BlockTag;
use steel_registry::{REGISTRY, blocks::BlockRef};
use steel_utils::{BlockPos, BlockStateId, Direction, Downcast as _};

use super::chest_block::ChestBlock;
use crate::behavior::block::{
    BlockBehavior, BlockEntityCreation, schedule_water_tick_if_waterlogged,
};
use crate::behavior::blocks::WeatheringCopper;
use crate::behavior::{BlockHitResult, BlockPlaceContext, InteractionResult, InventoryAccess};
use crate::behavior::{waxables, weathering};
use crate::block_entity::{BlockEntity as _, entities::ChestBlockEntity};
use crate::entity::ai::path::PathComputationType;
use crate::player::Player;
use crate::world::{ScheduledTickAccess, World};

/// Waxed copper chest.
#[block_behavior]
pub struct CopperChestBlock {
    block: BlockRef,
    #[json_arg(sound_events)]
    open_sound: SoundEventRef,
    #[json_arg(sound_events)]
    close_sound: SoundEventRef,
}

impl CopperChestBlock {
    /// Creates a copper chest with its extracted hinge sounds.
    pub const fn new(
        block: BlockRef,
        open_sound: SoundEventRef,
        close_sound: SoundEventRef,
    ) -> Self {
        Self {
            block,
            open_sound,
            close_sound,
        }
    }
}

/// Weathering copper chest.
#[block_behavior]
pub struct WeatheringCopperChestBlock {
    block: BlockRef,
    #[json_arg(sound_events)]
    open_sound: SoundEventRef,
    #[json_arg(sound_events)]
    close_sound: SoundEventRef,
}

impl WeatheringCopperChestBlock {
    /// Creates a weathering copper chest with its extracted hinge sounds.
    pub const fn new(
        block: BlockRef,
        open_sound: SoundEventRef,
        close_sound: SoundEventRef,
    ) -> Self {
        Self {
            block,
            open_sound,
            close_sound,
        }
    }
}

macro_rules! impl_copper_chest {
    ($chest:ty) => {
        impl BlockBehavior for $chest {
            fn chest_sounds(&self) -> Option<(SoundEventRef, SoundEventRef)> {
                Some((self.open_sound, self.close_sound))
            }

            fn get_state_for_placement(
                &self,
                context: &BlockPlaceContext<'_>,
            ) -> Option<BlockStateId> {
                let chest = ChestBlock::new(self.block, self.open_sound, self.close_sound);
                let state = chest.get_state_for_placement(context)?;
                Some(normalize_placement(state, context))
            }

            fn update_shape(
                &self,
                state: BlockStateId,
                world: &dyn ScheduledTickAccess,
                pos: BlockPos,
                direction: Direction,
                _neighbor_pos: BlockPos,
                neighbor_state: BlockStateId,
            ) -> BlockStateId {
                schedule_water_tick_if_waterlogged(state, world, pos);
                update_shape(state, direction, neighbor_state)
            }

            fn use_without_item(
                &self,
                state: BlockStateId,
                world: &Arc<World>,
                pos: BlockPos,
                player: &Player,
                hit: &BlockHitResult,
                inventory: &mut InventoryAccess,
            ) -> InteractionResult {
                ChestBlock::new(self.block, self.open_sound, self.close_sound)
                    .use_without_item(state, world, pos, player, hit, inventory)
            }

            fn new_block_entity(
                &self,
                world: Weak<World>,
                pos: BlockPos,
                state: BlockStateId,
            ) -> BlockEntityCreation {
                ChestBlock::new(self.block, self.open_sound, self.close_sound)
                    .new_block_entity(world, pos, state)
            }

            fn has_analog_output_signal(&self, state: BlockStateId) -> bool {
                ChestBlock::new(self.block, self.open_sound, self.close_sound)
                    .has_analog_output_signal(state)
            }

            fn get_analog_output_signal(
                &self,
                state: BlockStateId,
                world: &World,
                pos: BlockPos,
                direction: Direction,
            ) -> i32 {
                ChestBlock::new(self.block, self.open_sound, self.close_sound)
                    .get_analog_output_signal(state, world, pos, direction)
            }

            fn affect_neighbors_after_removal(
                &self,
                state: BlockStateId,
                world: &Arc<World>,
                pos: BlockPos,
                moved: bool,
            ) {
                ChestBlock::new(self.block, self.open_sound, self.close_sound)
                    .affect_neighbors_after_removal(state, world, pos, moved);
            }

            fn random_tick(&self, state: BlockStateId, world: &Arc<World>, pos: BlockPos) {
                let Some(age) = weathering::get_weather_state(self.block) else {
                    return;
                };
                if state.get_value(&BlockStateProperties::CHEST_TYPE) == ChestType::Right {
                    return;
                }
                let Some(entity) = world.get_block_entity(pos) else {
                    return;
                };
                if entity
                    .downcast_ref::<ChestBlockEntity>()
                    .is_some_and(|chest| !chest.has_openers())
                {
                    WeatheringCopper::new(age).change_over_time(state, world, pos);
                }
            }

            fn tick(&self, state: BlockStateId, world: &Arc<World>, pos: BlockPos) {
                ChestBlock::new(self.block, self.open_sound, self.close_sound)
                    .tick(state, world, pos);
            }

            fn trigger_event(
                &self,
                state: BlockStateId,
                world: &Arc<World>,
                pos: BlockPos,
                a: i32,
                b: i32,
            ) -> bool {
                ChestBlock::new(self.block, self.open_sound, self.close_sound)
                    .trigger_event(state, world, pos, a, b)
            }

            fn is_pathfindable(&self, state: BlockStateId, kind: PathComputationType) -> bool {
                ChestBlock::new(self.block, self.open_sound, self.close_sound)
                    .is_pathfindable(state, kind)
            }
        }
    };
}

impl_copper_chest!(CopperChestBlock);
impl_copper_chest!(WeatheringCopperChestBlock);

fn normalize_placement(state: BlockStateId, context: &BlockPlaceContext<'_>) -> BlockStateId {
    if state.get_value(&BlockStateProperties::CHEST_TYPE) == ChestType::Single {
        return state;
    }
    let neighbor = context
        .world
        .get_block_state(context.place_pos().relative(connected_direction(state)));
    let block = state.get_block();
    let other = neighbor.get_block();
    let normal = waxables::get_normal_from_waxed_variant(block).unwrap_or(block);
    let other_normal = waxables::get_normal_from_waxed_variant(other).unwrap_or(other);
    let (Some(age), Some(other_age)) = (
        weathering::get_weather_state(normal),
        weathering::get_weather_state(other_normal),
    ) else {
        return state;
    };
    let least = if age <= other_age {
        normal
    } else {
        other_normal
    };
    let both_waxed = normal != block && other_normal != other;
    let selected = if both_waxed {
        waxables::get_waxed_from_normal_variant(least).unwrap_or(least)
    } else {
        least
    };
    REGISTRY.blocks.copy_matching_properties(state, selected)
}

fn update_shape(
    mut state: BlockStateId,
    direction: Direction,
    neighbor_state: BlockStateId,
) -> BlockStateId {
    let neighbor_type = neighbor_state
        .get_block()
        .has_tag(&BlockTag::COPPER_CHESTS)
        .then(|| neighbor_state.try_get_value(&BlockStateProperties::CHEST_TYPE))
        .flatten();
    if let Some(neighbor_type) = &neighbor_type
        && direction.is_horizontal()
    {
        if state.get_value(&BlockStateProperties::CHEST_TYPE) == ChestType::Single
            && *neighbor_type != ChestType::Single
            && state.get_value(&BlockStateProperties::HORIZONTAL_FACING)
                == neighbor_state.get_value(&BlockStateProperties::HORIZONTAL_FACING)
            && connected_direction(neighbor_state) == direction.opposite()
        {
            let opposite_type = match neighbor_type {
                ChestType::Left => ChestType::Right,
                ChestType::Right => ChestType::Left,
                ChestType::Single => ChestType::Single,
            };
            state = state.set_value(&BlockStateProperties::CHEST_TYPE, opposite_type);
        }
    } else if connected_direction(state) == direction {
        state = state.set_value(&BlockStateProperties::CHEST_TYPE, ChestType::Single);
    }

    if neighbor_type.is_some()
        && state.get_value(&BlockStateProperties::CHEST_TYPE) != ChestType::Single
        && connected_direction(state) == direction
    {
        return neighbor_state.with_properties_of(state);
    }
    state
}

fn connected_direction(state: BlockStateId) -> Direction {
    let facing = state.get_value(&BlockStateProperties::HORIZONTAL_FACING);
    if state.get_value(&BlockStateProperties::CHEST_TYPE) == ChestType::Left {
        facing.rotate_y_clockwise()
    } else {
        facing.rotate_y_counter_clockwise()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::behavior::init_behaviors;
    use crate::block_entity::init_block_entities;
    use crate::inventory::lock::ContainerLockGuard;
    use crate::test_support::{
        TestPlayerBuilder, fresh_test_world, insert_ready_full_chunk, use_block,
    };
    use steel_registry::{
        init_vanilla_registry, item_stack::ItemStack, sound_events, vanilla_blocks, vanilla_items,
    };
    use steel_utils::{ChunkPos, types::UpdateFlags};

    #[test]
    fn copper_chest_pairs_with_other_oxidation_and_opens_saved_inventory() {
        init_vanilla_registry();
        init_behaviors();
        init_block_entities();
        let fixture = fresh_test_world("copper_chest_interaction");
        let world = &fixture.world;
        insert_ready_full_chunk(world, ChunkPos::new(0, 0));
        let pos = BlockPos::new(5, 64, 5);
        let neighbor = vanilla_blocks::EXPOSED_COPPER_CHEST
            .default_state()
            .set_value(&BlockStateProperties::HORIZONTAL_FACING, Direction::North);
        assert!(world.set_block(pos.east(), neighbor, UpdateFlags::UPDATE_NONE));

        let behavior = CopperChestBlock::new(
            &vanilla_blocks::WAXED_COPPER_CHEST,
            &sound_events::BLOCK_COPPER_CHEST_OPEN,
            &sound_events::BLOCK_COPPER_CHEST_CLOSE,
        );
        let mut item = ItemStack::new(&vanilla_items::WAXED_COPPER_CHEST);
        let context =
            BlockPlaceContext::directional(world, pos, Direction::South, &mut item, Direction::Up);
        let placed = behavior
            .get_state_for_placement(&context)
            .expect("placement");
        assert_eq!(placed.get_block(), &vanilla_blocks::COPPER_CHEST);
        assert_eq!(
            placed.get_value(&BlockStateProperties::CHEST_TYPE),
            ChestType::Left
        );
        assert!(world.set_block(pos, placed, UpdateFlags::UPDATE_ALL));

        let state = world.get_block_state(pos);
        let chest = ChestBlock::get_container(state, world, pos, false).expect("copper chest");
        assert_eq!(chest.containers.len(), 2);
        let container = &chest.containers[0];
        let mut guard = ContainerLockGuard::lock_all(&[container]).expect("container lock");
        assert!(guard.set_item(
            container.container_id(),
            0,
            ItemStack::new(&vanilla_items::DIAMOND)
        ));
        drop(guard);
        let owner = &chest.owners[0];
        let saved = owner.save_custom_only();
        owner
            .load_with_owned_components(&saved)
            .expect("reload chest");
        assert_eq!(owner.save_custom_only(), saved);

        let player = TestPlayerBuilder::new(Arc::clone(world), "Opener", 1).build();
        player.set_client_loaded(true);
        use_block(world, pos, &player);
        assert!(player.has_container_open());
    }
}
