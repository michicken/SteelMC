//! Armor stand item placement.

use glam::DVec3;
use steel_macros::item_behavior;
use steel_protocol::packets::game::SoundSource;
use steel_registry::item_stack::ItemStack;
use steel_registry::{sound_events, vanilla_entities, vanilla_game_events};
use steel_utils::{Direction, WorldAabb};

use crate::behavior::{InteractionResult, ItemBehavior, UseOnContext};
use crate::entity::{
    Entity as _, EntitySpawnPlacement, EntitySpawnReason, EntitySpawnRequest, spawn_entity,
};
use crate::physics::{CollisionWorld as _, WorldCollisionProvider};

/// Places an armor stand.
#[item_behavior(class = "ArmorStandItem")]
pub struct EntityPlaceItem;

impl ItemBehavior for EntityPlaceItem {
    fn use_on(&self, context: &mut UseOnContext) -> InteractionResult {
        if context.hit_result.direction == Direction::Down {
            return InteractionResult::Fail;
        }
        let pos = context.build_place_context().place_pos();
        let dimensions = &vanilla_entities::ARMOR_STAND.dimensions;
        let center = DVec3::new(
            f64::from(pos.x()) + 0.5,
            f64::from(pos.y()),
            f64::from(pos.z()) + 0.5,
        );
        let half_width = f64::from(dimensions.width) / 2.0;
        let bounds = WorldAabb::new(
            center.x - half_width,
            center.y,
            center.z - half_width,
            center.x + half_width,
            center.y + f64::from(dimensions.height),
            center.z + half_width,
        );
        if WorldCollisionProvider::new(context.world).has_block_collision(&bounds)
            || context.world.has_entity_in_aabb_matching(&bounds, |_| true)
        {
            return InteractionResult::Fail;
        }
        let stack = context.inv.with_item(|stack| stack.clone());
        let request = EntitySpawnRequest {
            entity_type: &vanilla_entities::ARMOR_STAND,
            placement: EntitySpawnPlacement::Block {
                pos,
                try_move_down: true,
                moved_up: true,
            },
            reason: EntitySpawnReason::SpawnItemUse,
            finalize_spawn: false,
            play_ambient_sound: false,
            item_stack: Some(&stack),
            user_is_operator: context.player.is_operator(),
        };
        let Ok(entity) = spawn_entity(context.world, request) else {
            return InteractionResult::Fail;
        };
        let yaw = steel_math::wrap_degrees(context.player.rotation().0 - 180.0);
        const ROTATION_STEP_DEGREES: f32 = 45.0;
        let snapped_yaw = ((yaw + ROTATION_STEP_DEGREES / 2.0) / ROTATION_STEP_DEGREES).floor()
            * ROTATION_STEP_DEGREES;
        entity.set_rotation((snapped_yaw, 0.0));
        context.world.play_sound_at(
            &sound_events::ENTITY_ARMOR_STAND_PLACE,
            SoundSource::Blocks,
            entity.position(),
            0.75,
            0.8,
            None,
        );
        entity.game_event_with_source_entity(
            &vanilla_game_events::ENTITY_PLACE,
            Some(context.player.as_ref()),
        );
        context.inv.with_item(ItemStack::shrink_one);
        InteractionResult::Success
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::behavior::BlockHitResult;
    use crate::bootstrap::init_globals;
    use crate::entity::{Entity as _, next_entity_id};
    use crate::player::game_mode::use_item_on;
    use crate::test_support::{TestPlayerBuilder, fresh_test_world, insert_ready_full_chunk};
    use std::sync::Arc;
    use steel_registry::{
        data_components::vanilla_components::CUSTOM_NAME, vanilla_blocks, vanilla_items,
    };
    use steel_utils::{
        BlockPos, ChunkPos,
        types::{GameType, InteractionHand, UpdateFlags},
    };
    use text_components::TextComponent;

    #[test]
    fn armor_stand_item_places_named_entity_from_the_registered_use_path() {
        init_globals();
        let fixture = fresh_test_world("armor_stand_item_use");
        let world = &fixture.world;
        insert_ready_full_chunk(world, ChunkPos::new(0, 0));
        let pos = BlockPos::new(8, 64, 8);
        assert!(world.set_block(
            pos,
            vanilla_blocks::STONE.default_state(),
            UpdateFlags::UPDATE_NONE
        ));
        let player =
            TestPlayerBuilder::new(Arc::clone(world), "StandPlayer", next_entity_id()).build();
        player.restore_game_modes(GameType::Survival, None);
        let mut item = ItemStack::new(&vanilla_items::ARMOR_STAND);
        item.set(CUSTOM_NAME, TextComponent::plain("Named stand"));
        player
            .inventory
            .lock()
            .set_item_in_hand(InteractionHand::MainHand, item);
        let hit = BlockHitResult {
            location: DVec3::new(8.5, 65.0, 8.5),
            direction: Direction::Up,
            block_pos: pos,
            miss: false,
            inside: false,
            world_border_hit: false,
        };
        assert_eq!(
            use_item_on(&player, world, InteractionHand::MainHand, &hit),
            InteractionResult::Success
        );
        let bounds = WorldAabb::new(8.0, 65.0, 8.0, 9.0, 67.0, 9.0);
        let stands = world.get_entities_in_aabb_matching(&bounds, |entity| {
            entity.entity_type() == &vanilla_entities::ARMOR_STAND
        });
        assert_eq!(stands.len(), 1);
        assert_eq!(
            stands[0].custom_name(),
            Some(TextComponent::plain("Named stand"))
        );
        assert!(
            player
                .inventory
                .lock()
                .get_item_in_hand(InteractionHand::MainHand)
                .is_empty()
        );
    }
}
