//! Painting item (HangingEntityItem for paintings).

use std::sync::Arc;

use glam::DVec3;
use steel_registry::blocks::block_state_ext::BlockStateExt as _;
use steel_registry::entity_type::EntityTypeRef;
use steel_registry::item_stack::ItemStack;
use steel_registry::vanilla_game_events;
use steel_utils::Direction;

use crate::behavior::{InteractionResult, ItemBehavior, UseOnContext};
use crate::entity::entities::PaintingEntity;
use crate::entity::next_entity_id;
use crate::world::game_event::GameEventContext;

pub struct PaintingItem {
    entity_type: EntityTypeRef,
}

impl PaintingItem {
    #[must_use]
    pub const fn new(entity_type: EntityTypeRef) -> Self {
        Self { entity_type }
    }
}

impl ItemBehavior for PaintingItem {
    fn use_on(&self, context: &mut UseOnContext) -> InteractionResult {
        let world = context.world;
        let hit = &context.hit_result;
        let face = hit.direction;
        if !matches!(face, Direction::North | Direction::South | Direction::East | Direction::West) {
            return InteractionResult::Fail;
        }
        let clicked = hit.block_pos;
        if world.get_block_state(clicked).is_air() {
            return InteractionResult::Fail;
        }
        let hanging_pos = clicked.relative(face);
        if !world.get_block_state(hanging_pos).is_air() {
            return InteractionResult::Fail;
        }

        let position = match face {
            Direction::North => DVec3::new(f64::from(clicked.x()) + 0.5, f64::from(clicked.y()) + 0.5, f64::from(clicked.z()) - 0.5),
            Direction::South => DVec3::new(f64::from(clicked.x()) + 0.5, f64::from(clicked.y()) + 0.5, f64::from(clicked.z()) + 1.5),
            Direction::East => DVec3::new(f64::from(clicked.x()) + 1.5, f64::from(clicked.y()) + 0.5, f64::from(clicked.z()) + 0.5),
            Direction::West => DVec3::new(f64::from(clicked.x()) - 0.5, f64::from(clicked.y()) + 0.5, f64::from(clicked.z()) + 0.5),
            _ => unreachable!(),
        };

        let painting = PaintingEntity::new_with_direction(
            self.entity_type,
            next_entity_id(),
            position,
            Arc::downgrade(world),
            face,
        );
        if let Err(error) = world.try_add_entity(Arc::new(painting)) {
            log::warn!("Failed to place painting at {hanging_pos:?}: {error}");
            return InteractionResult::Fail;
        }
        world.game_event(
            &vanilla_game_events::ENTITY_PLACE,
            hanging_pos,
            &GameEventContext::new(Some(context.player.as_ref()), None),
        );
        context.inv.with_item(ItemStack::shrink_one);
        InteractionResult::Success
    }
}
