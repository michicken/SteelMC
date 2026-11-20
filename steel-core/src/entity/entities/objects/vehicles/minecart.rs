//! Vanilla rideable minecart.

use std::sync::{Arc, Weak};

use glam::DVec3;
use simdnbt::borrow::NbtCompound as BorrowedNbtCompoundView;
use simdnbt::owned::NbtCompound;

use steel_macros::entity_behavior;
use steel_registry::entity_type::EntityTypeRef;
use steel_registry::fluid::FluidState;
use steel_registry::vanilla_blocks;
use steel_registry::vanilla_entity_data::MinecartEntityData;
use steel_registry::{items::ItemRef, vanilla_entities, vanilla_items};
use steel_utils::axis::Axis;
use steel_utils::block_util::FoundRectangle;
use steel_utils::locks::SyncMutex;
use steel_utils::types::InteractionHand;
use steel_utils::{BlockPos, BlockStateId, Direction, DowncastType, DowncastTypeKey};

use super::abstract_minecart::{
    AbstractMinecart, AbstractMinecartBase, minecart_relative_portal_position,
};
use super::minecart_behavior::MinecartBehavior;
use super::minecart_container::MinecartContainer;
use super::tnt_minecart::TntMinecart;
use super::vehicle_entity::{
    VEHICLE_DIMENSION_CHANGING_DELAY, VEHICLE_HURT_DURATION_TICKS, VehicleEntity,
    vehicle_ignores_explosion,
};
use crate::behavior::InteractionResult;
use crate::entity::{
    DamageSource, Entity, EntityBase, EntityBaseLoad, EntityEventSource, EntityMovementEmission,
    EntitySyncedData,
};
use crate::physics::{MoveResult, MoverType};
use crate::player::Player;
use crate::world::{Explosion, World};

const ACTIVATOR_RAIL_DAMAGE: f32 = 50.0;

/// A minecart entity.
#[entity_behavior(
    class = "Minecart",
    class_alias = "MinecartTNT",
    class_alias = "MinecartChest"
)]
pub struct MinecartEntity {
    base: EntityBase,
    entity_type: EntityTypeRef,
    entity_data: SyncMutex<MinecartEntityData>,
    minecart_base: AbstractMinecartBase,
    tnt: Option<Box<TntMinecart>>,
    pub(super) chest: Option<Box<MinecartContainer>>,
}

// SAFETY: This key is owned by Steel and uniquely identifies `MinecartEntity`.
unsafe impl DowncastType for MinecartEntity {
    const TYPE_KEY: DowncastTypeKey = DowncastTypeKey::new("steel:entity/minecart");
}

impl MinecartEntity {
    /// Creates a new minecart entity.
    #[must_use]
    pub fn new(entity_type: EntityTypeRef, id: i32, position: DVec3, world: Weak<World>) -> Self {
        Self {
            base: EntityBase::new(id, position, entity_type.dimensions, world),
            entity_type,
            entity_data: SyncMutex::new(MinecartEntityData::new()),
            minecart_base: AbstractMinecartBase::new(),
            tnt: (entity_type == &vanilla_entities::TNT_MINECART)
                .then(|| Box::new(TntMinecart::new())),
            chest: (entity_type == &vanilla_entities::CHEST_MINECART)
                .then(|| Box::new(MinecartContainer::new())),
        }
    }

    /// Assigns the chest's deferred structure loot.
    pub fn set_loot_table(&self, table: steel_utils::Identifier, seed: i64) {
        if let Some(chest) = &self.chest {
            chest.set_loot(table, seed);
        }
    }

    /// Creates a minecart entity from saved data.
    #[must_use]
    pub fn from_saved(entity_type: EntityTypeRef, load: EntityBaseLoad) -> Self {
        Self {
            base: EntityBase::from_load(load, entity_type.dimensions),
            entity_type,
            entity_data: SyncMutex::new(MinecartEntityData::new()),
            minecart_base: AbstractMinecartBase::new(),
            tnt: (entity_type == &vanilla_entities::TNT_MINECART)
                .then(|| Box::new(TntMinecart::new())),
            chest: (entity_type == &vanilla_entities::CHEST_MINECART)
                .then(|| Box::new(MinecartContainer::new())),
        }
    }
}

impl VehicleEntity for MinecartEntity {
    fn hurt_time(&self) -> i32 {
        *self.entity_data.lock().vehicle_entity().id_hurt.get()
    }

    fn set_hurt_time(&self, value: i32) {
        self.entity_data
            .lock()
            .vehicle_entity_mut()
            .id_hurt
            .set(value);
    }

    fn hurt_dir(&self) -> i32 {
        *self.entity_data.lock().vehicle_entity().id_hurtdir.get()
    }

    fn set_hurt_dir(&self, value: i32) {
        self.entity_data
            .lock()
            .vehicle_entity_mut()
            .id_hurtdir
            .set(value);
    }

    fn damage(&self) -> f32 {
        *self.entity_data.lock().vehicle_entity().id_damage.get()
    }

    fn set_damage(&self, value: f32) {
        self.entity_data
            .lock()
            .vehicle_entity_mut()
            .id_damage
            .set(value);
    }

    fn vehicle_drop_item(&self) -> ItemRef {
        // The generic minecart implementation backs every minecart variant.
        // Keep the dropped item matched to the entity type instead of always
        // returning a plain minecart.
        if self.entity_type == &vanilla_entities::CHEST_MINECART {
            &vanilla_items::CHEST_MINECART
        } else if self.entity_type == &vanilla_entities::FURNACE_MINECART {
            &vanilla_items::FURNACE_MINECART
        } else if self.entity_type == &vanilla_entities::TNT_MINECART {
            &vanilla_items::TNT_MINECART
        } else if self.entity_type == &vanilla_entities::HOPPER_MINECART {
            &vanilla_items::HOPPER_MINECART
        } else if self.entity_type == &vanilla_entities::COMMAND_BLOCK_MINECART {
            &vanilla_items::COMMAND_BLOCK_MINECART
        } else {
            &vanilla_items::MINECART
        }
    }

    fn should_source_destroy(&self, source: &DamageSource) -> bool {
        self.tnt.is_some() && TntMinecart::damage_ignites(source)
    }

    fn vehicle_destroy(&self, world: &World, source: &DamageSource) {
        if let Some(tnt) = &self.tnt
            && tnt.handle_destroy(self, world, source)
        {
            return;
        }
        self.vehicle_destroy_with_item(world, self.vehicle_drop_item());
    }
}

impl AbstractMinecart for MinecartEntity {
    fn minecart_base(&self) -> &AbstractMinecartBase {
        &self.minecart_base
    }

    fn is_rideable(&self) -> bool {
        self.tnt.is_none() && self.chest.is_none()
    }

    fn default_display_block_state(&self) -> BlockStateId {
        if self.tnt.is_some() {
            vanilla_blocks::TNT.default_state()
        } else if self.chest.is_some() {
            vanilla_blocks::CHEST.default_state()
        } else {
            vanilla_blocks::AIR.default_state()
        }
    }

    fn default_display_offset(&self) -> i32 {
        if self.chest.is_some() { 8 } else { 6 }
    }

    fn apply_natural_slowdown(&self, movement: DVec3) -> DVec3 {
        let Some(chest) = &self.chest else {
            return self.minecart_natural_slowdown(movement);
        };
        chest.slowdown(self, movement)
    }

    fn custom_display_block(&self) -> Option<BlockStateId> {
        *self
            .entity_data
            .lock()
            .abstract_minecart()
            .id_custom_display_block
            .get()
    }

    fn set_custom_display_block(&self, value: Option<BlockStateId>) {
        self.entity_data
            .lock()
            .abstract_minecart_mut()
            .id_custom_display_block
            .set(value);
    }

    fn display_offset(&self) -> i32 {
        let data = self.entity_data.lock();
        let minecart = data.abstract_minecart();
        if minecart.id_custom_display_block.get().is_none() {
            return self.default_display_offset();
        }
        *minecart.id_display_offset.get()
    }

    fn set_display_offset(&self, value: i32) {
        self.entity_data
            .lock()
            .abstract_minecart_mut()
            .id_display_offset
            .set(value);
    }

    fn activate_minecart(&self, world: &World, _pos: BlockPos, powered: bool) {
        if !powered {
            return;
        }
        if let Some(tnt) = &self.tnt {
            tnt.prime(self, world, None);
            return;
        }
        if self.chest.is_some() {
            return;
        }
        if self.is_vehicle() {
            self.eject_passengers();
        }
        if self.hurt_time() == 0 {
            self.set_hurt_dir(-self.hurt_dir());
            self.set_hurt_time(VEHICLE_HURT_DURATION_TICKS);
            self.set_damage(ACTIVATOR_RAIL_DAMAGE);
            self.mark_hurt();
        }
    }
}

impl Entity for MinecartEntity {
    fn interact(
        &self,
        player: &Player,
        _hand: InteractionHand,
        _location: DVec3,
    ) -> InteractionResult {
        if let Some(chest) = &self.chest {
            return chest.interact(self, player);
        }
        if !self.is_rideable() || player.is_secondary_use_active() || self.is_vehicle() {
            return InteractionResult::Pass;
        }

        let Some(world) = self.level() else {
            return InteractionResult::Pass;
        };
        let Some(minecart) = world.get_entity_by_id(self.id()) else {
            return InteractionResult::Pass;
        };

        if player.start_riding(&minecart) {
            InteractionResult::SuccessServer
        } else {
            InteractionResult::Consume
        }
    }

    fn container_ref(&self) -> Option<crate::inventory::lock::ContainerRef> {
        let chest = self.chest.as_ref()?;
        let entity = self.level()?.get_entity_by_id(self.id())?;
        Some(chest.container_ref(&entity))
    }

    fn set_removed(&self, reason: crate::entity::RemovalReason) {
        if let Some(chest) = &self.chest {
            chest.remove(self, reason);
        } else {
            self.base.set_removed(reason);
        }
    }

    fn base(&self) -> &EntityBase {
        &self.base
    }

    fn entity_type(&self) -> EntityTypeRef {
        self.entity_type
    }

    fn tick(self: Arc<Self>) {
        let entity = Arc::clone(&self).into_shared_entity();
        if let Some(minecart) = entity.as_abstract_minecart() {
            minecart.tick_minecart(&entity);
        }
        if let Some(tnt) = &self.tnt
            && !self.is_removed()
        {
            tnt.tick(&self, &entity);
        }
    }

    fn get_default_gravity(&self) -> f64 {
        self.minecart_default_gravity()
    }

    fn get_air_drag(&self) -> f64 {
        self.minecart_air_drag()
    }

    fn block_speed_factor(&self) -> f32 {
        self.minecart_block_speed_factor()
    }

    fn move_entity(self: Arc<Self>, mover_type: MoverType, delta: DVec3) -> Option<MoveResult> {
        self.minecart_move_entity(mover_type, delta)
    }

    fn apply_effects_from_blocks(&self) {
        self.minecart_apply_effects_from_blocks();
    }

    fn known_movement(&self) -> DVec3 {
        MinecartBehavior::known_movement(self.default_known_movement())
    }

    fn motion_direction(&self) -> Direction {
        MinecartBehavior::motion_direction(self)
    }

    fn is_on_rails(&self) -> bool {
        self.minecart_base().on_rails()
    }

    fn is_pickable(&self) -> bool {
        self.minecart_is_pickable()
    }

    fn is_pushable(&self) -> bool {
        self.minecart_is_pushable()
    }

    fn movement_emission(&self) -> EntityMovementEmission {
        self.minecart_movement_emission()
    }

    fn blocks_building(&self) -> bool {
        self.minecart_blocks_building()
    }

    fn dimension_changing_delay(&self) -> i32 {
        VEHICLE_DIMENSION_CHANGING_DELAY
    }

    fn get_relative_portal_position(&self, axis: Axis, area: FoundRectangle) -> DVec3 {
        minecart_relative_portal_position(self, axis, area)
    }

    fn passenger_attachment_point(&self, passenger: &dyn Entity) -> DVec3 {
        self.minecart_passenger_attachment_point(passenger)
    }

    fn can_collide_with(&self, other: &dyn Entity) -> bool {
        self.minecart_can_collide_with(other)
    }

    fn push_entity(&self, entity: &dyn Entity) {
        self.minecart_push_entity(entity);
    }

    fn hurt(&self, world: &World, source: &DamageSource, amount: f32) -> bool {
        if let Some(tnt) = &self.tnt {
            tnt.handle_projectile_hit(self, world, source);
        }
        self.vehicle_hurt(world, source, amount)
    }

    fn cause_fall_damage(
        self: Arc<Self>,
        fall_distance: f64,
        damage_modifier: f32,
        source: &DamageSource,
    ) -> bool {
        if let Some(tnt) = &self.tnt {
            tnt.handle_fall(&self, fall_distance);
        }
        if !self.is_fall_damage_immune() {
            self.propagate_fall_to_passengers(fall_distance, damage_modifier, source);
        }
        false
    }

    fn block_explosion_resistance(
        &self,
        _explosion: &dyn Explosion,
        world: &World,
        pos: BlockPos,
        state: BlockStateId,
        _fluid: FluidState,
        resistance: f32,
    ) -> f32 {
        if self
            .tnt
            .as_ref()
            .is_some_and(|tnt| tnt.protects_rail(world, pos, state))
        {
            0.0
        } else {
            resistance
        }
    }

    fn should_block_explode(
        &self,
        _explosion: &dyn Explosion,
        world: &World,
        pos: BlockPos,
        state: BlockStateId,
        _power: f32,
    ) -> bool {
        !self
            .tnt
            .as_ref()
            .is_some_and(|tnt| tnt.protects_rail(world, pos, state))
    }

    fn ignore_explosion(&self, explosion: &dyn Explosion) -> bool {
        vehicle_ignores_explosion(explosion)
    }

    fn synced_data(&self) -> Option<&dyn EntitySyncedData> {
        Some(&self.entity_data)
    }

    fn save_additional(&self, nbt: &mut NbtCompound) {
        self.save_abstract_minecart_data(nbt);
        if let Some(chest) = &self.chest {
            chest.save(nbt);
        }
        if let Some(tnt) = &self.tnt {
            tnt.save(nbt);
        }
    }

    fn load_additional(&self, nbt: BorrowedNbtCompoundView<'_, '_>) {
        self.load_abstract_minecart_data(nbt);
        if let Some(chest) = &self.chest {
            chest.load(nbt);
        }
        if let Some(tnt) = &self.tnt {
            tnt.load(nbt);
        }
    }

    // TODO: Implement getPickResult.
}

#[cfg(test)]
#[path = "tests/minecart.rs"]
mod tests;
