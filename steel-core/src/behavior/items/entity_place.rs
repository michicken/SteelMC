//! Generic placement behavior for block-sized entities such as armor stands.

use std::sync::Arc;

use glam::DVec3;
use steel_registry::blocks::block_state_ext::BlockStateExt as _;
use steel_registry::entity_type::EntityTypeRef;
use steel_registry::item_stack::ItemStack;
use steel_registry::vanilla_game_events;
use steel_utils::WorldAabb;

use crate::behavior::{InteractionResult, ItemBehavior, UseOnContext};
use crate::entity::{create_entity_instance, next_entity_id};
use crate::world::game_event::GameEventContext;

/// Places a non-living entity centered on the block above the clicked face.
pub struct EntityPlaceItem {
    entity_type: EntityTypeRef,
}

impl EntityPlaceItem {
    #[must_use]
    pub const fn new(entity_type: EntityTypeRef) -> Self {
        Self { entity_type }
    }
}

impl ItemBehavior for EntityPlaceItem {
    fn use_on(&self, context: &mut UseOnContext) -> InteractionResult {
        let world = context.world;
        let clicked = context.hit_result.block_pos;
        let above = clicked.above();
        if !world.get_block_state(above).is_air() {
            return InteractionResult::Fail;
        }

        let x = f64::from(above.x());
        let y = f64::from(above.y());
        let z = f64::from(above.z());
        let bounds = WorldAabb::new(x, y, z, x + 1.0, y + 2.0, z + 1.0);
        if world.has_entity_in_aabb_matching(&bounds, |entity| !entity.is_spectator()) {
            return InteractionResult::Fail;
        }

        let entity = match create_entity_instance(
            world,
            self.entity_type,
            DVec3::new(x + 0.5, y, z + 0.5),
        ) {
            Ok(entity) => entity,
            Err(error) => {
                log::warn!(
                    "Failed to create placed entity {}: {error:?}",
                    self.entity_type.key
                );
                return InteractionResult::Fail;
            }
        };
        if let Err(error) = world.try_add_entity(entity) {
            log::warn!(
                "Failed to add placed entity {}: {error}",
                self.entity_type.key
            );
            return InteractionResult::Fail;
        }

        world.game_event(
            &vanilla_game_events::ENTITY_PLACE,
            above,
            &GameEventContext::new(Some(context.player.as_ref()), None),
        );
        context.inv.with_item(ItemStack::shrink_one);
        InteractionResult::Success
    }
}
