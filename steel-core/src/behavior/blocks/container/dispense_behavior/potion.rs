use glam::DVec3;
use std::sync::Arc;
use steel_registry::blocks::block_state_ext::BlockStateExt;
use steel_registry::blocks::properties::BlockStateProperties;
use steel_registry::data_components::PotionContents;
use steel_registry::data_components::vanilla_components::POTION_CONTENTS;
use steel_registry::fluid::FluidStateExt;
use steel_registry::item_stack::ItemStack;
use steel_registry::particle_type::ParticleData;
use steel_registry::vanilla_block_tags::BlockTag;
use steel_registry::{
    sound_events, vanilla_blocks, vanilla_game_events, vanilla_items, vanilla_particle_types,
    vanilla_potions,
};
use steel_utils::BlockPos;
use steel_utils::BlockStateId;
use steel_utils::types::UpdateFlags;

use super::DefaultDispenseBehavior;
use super::DispenseItemBehavior;
use crate::behavior::blocks::container::dispenser_block::FACING;
use crate::world::World;
use crate::world::game_event::GameEventContext;

const MUD_SPLASH_PARTICLE_COUNT: usize = 5;

pub struct GlassBottleDispenseBehavior;

impl DispenseItemBehavior for GlassBottleDispenseBehavior {
    fn dispense(
        &self,
        world: &Arc<World>,
        pos: BlockPos,
        state: BlockStateId,
        item: ItemStack,
    ) -> ItemStack {
        let target_pos = pos.relative(state.get_value(FACING));
        // TODO: honey bottle from a beehive with honey_level >= 5 — needs
        // beehive honey-level tracking, which isn't ported yet.
        if world
            .get_block_state(target_pos)
            .get_fluid_state()
            .is_water()
        {
            world.game_event(
                &vanilla_game_events::FLUID_PICKUP,
                pos,
                &GameEventContext::new(None, None),
            );
            return PotionContents::create_item_stack(
                &vanilla_items::POTION,
                &vanilla_potions::WATER,
            );
        }

        DefaultDispenseBehavior.dispense(world, pos, state, item)
    }
}

/// Water bottles convert the block in front into mud.
///
/// Vanilla plays the splash particles, sound, and game event at the dispenser
/// position rather than the converted block; this is kept for parity.
pub struct PotionDispenseBehavior;

impl DispenseItemBehavior for PotionDispenseBehavior {
    fn dispense(
        &self,
        world: &Arc<World>,
        pos: BlockPos,
        state: BlockStateId,
        item: ItemStack,
    ) -> ItemStack {
        if !item
            .get_or_default(POTION_CONTENTS, PotionContents::empty())
            .is(&vanilla_potions::WATER)
        {
            return DefaultDispenseBehavior.dispense(world, pos, state, item);
        }

        let target_pos = pos.relative(state.get_value(FACING));
        let target_state = world.get_block_state(target_pos);

        if target_state
            .get_block()
            .has_tag(&BlockTag::CONVERTIBLE_TO_MUD)
        {
            for _ in 0..MUD_SPLASH_PARTICLE_COUNT {
                world.send_particles(
                    ParticleData::simple(&vanilla_particle_types::SPLASH),
                    DVec3::new(
                        f64::from(pos.x()) + rand::random::<f64>(),
                        f64::from(pos.y()) + 1.0,
                        f64::from(pos.z()) + rand::random::<f64>(),
                    ),
                    1,
                    DVec3::ZERO,
                    1.0,
                );
            }
            world.play_block_sound(&sound_events::ITEM_BOTTLE_EMPTY, pos, 1.0, 1.0, None);
            world.game_event(
                &vanilla_game_events::FLUID_PLACE,
                pos,
                &GameEventContext::new(None, None),
            );
            world.set_block(
                target_pos,
                vanilla_blocks::MUD.default_state(),
                UpdateFlags::UPDATE_ALL,
            );

            return ItemStack::new(&vanilla_items::GLASS_BOTTLE);
        }

        if fill_water_cauldron(world, target_pos, target_state) {
            world.play_block_sound(&sound_events::ITEM_BOTTLE_EMPTY, pos, 1.0, 1.0, None);
            world.game_event(
                &vanilla_game_events::FLUID_PLACE,
                pos,
                &GameEventContext::new(None, None),
            );
            return ItemStack::new(&vanilla_items::GLASS_BOTTLE);
        }

        DefaultDispenseBehavior.dispense(world, pos, state, item)
    }
}

/// Adds one level of water to an empty or partially-filled water cauldron.
/// Returns `false` for full cauldrons and non-water cauldron contents.
fn fill_water_cauldron(world: &Arc<World>, pos: BlockPos, state: BlockStateId) -> bool {
    let level_prop = &BlockStateProperties::LEVEL_CAULDRON;
    let block = state.get_block();

    if block == &vanilla_blocks::CAULDRON {
        let new_state = vanilla_blocks::WATER_CAULDRON
            .default_state()
            .set_value(level_prop, 1);
        world.set_block(pos, new_state, UpdateFlags::UPDATE_ALL);
        return true;
    }

    if block == &vanilla_blocks::WATER_CAULDRON {
        let level: u8 = state.get_value(level_prop);
        if level < level_prop.max {
            world.set_block(
                pos,
                state.set_value(level_prop, level + 1),
                UpdateFlags::UPDATE_ALL,
            );
            return true;
        }
    }

    false
}
