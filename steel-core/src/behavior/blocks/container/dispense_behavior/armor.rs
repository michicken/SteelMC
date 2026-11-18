use std::sync::Arc;
use steel_protocol::packets::game::SoundSource;
use steel_registry::blocks::block_state_ext::BlockStateExt;
use steel_registry::item_stack::ItemStack;
use steel_registry::{level_events, sound_events};
use steel_utils::BlockPos;
use steel_utils::BlockStateId;
use steel_utils::WorldAabb;

use super::DispenseItemBehavior;
use crate::behavior::blocks::container::dispenser_block::FACING;
use crate::world::World;

pub struct ArmorDispenseBehavior;

impl DispenseItemBehavior for ArmorDispenseBehavior {
    fn dispense(
        &self,
        world: &Arc<World>,
        pos: BlockPos,
        state: BlockStateId,
        mut item: ItemStack,
    ) -> ItemStack {
        let facing = state.get_value(FACING);
        let target_pos = pos.relative(facing);
        let target_aabb = WorldAabb::new(
            f64::from(target_pos.x()),
            f64::from(target_pos.y()),
            f64::from(target_pos.z()),
            f64::from(target_pos.x()) + 1.0,
            f64::from(target_pos.y()) + 1.0,
            f64::from(target_pos.z()) + 1.0,
        );

        let entities = world.get_entities_in_aabb(&target_aabb);

        for entity in entities {
            if let Some(living) = entity.as_living_entity()
                && living.can_equip_with_dispenser(&item)
                && let Some(slot) = item.get_equippable().map(|equippable| equippable.slot)
            {
                // Vanilla equips one item and leaves the rest of the stack in the dispenser.
                let equipped = item.split(1);
                living.with_equipment_slot_mut(slot, &mut |current| {
                    *current = equipped.clone();
                });

                let default_sound = equipped
                    .get_equippable()
                    .and_then(|equippable| equippable.equip_sound.registry_ref());
                let sound = living
                    .equip_sound(slot, &equipped)
                    .or(default_sound)
                    .unwrap_or(&sound_events::ITEM_ARMOR_EQUIP_GENERIC);

                world.play_sound(sound, SoundSource::Blocks, target_pos, 1.0, 1.0, None);

                return item;
            }
        }

        // Fallback: eject exactly one item, like vanilla.
        let ejected = item.split(1);
        world.drop_item_stack(target_pos, ejected);
        world.level_event(level_events::SOUND_DISPENSER_DISPENSE, pos, 0, None);
        item
    }
}
