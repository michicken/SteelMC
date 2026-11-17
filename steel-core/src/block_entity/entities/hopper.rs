//! Hopper block entity.
//!
//! Vanilla parity notes:
//! - Five slots, transferred one item per activation (2.5 items/second while enabled).
//! - Transfer cooldown of [`MOVE_ITEM_SPEED`] game ticks between activations.
//! - Pushes into the container the hopper faces, then pulls from the container above.
//! - A hopper facing up does not pull, otherwise it would move an item up and immediately
//!   move it back down.
//!
//! Missing compared to Vanilla for now: pulling items from `ItemEntity` stacks resting on
//! top of the hopper (container-to-container transport only).

use std::sync::{Arc, Weak};

use simdnbt::borrow::BaseNbtCompound as BorrowedNbtCompound;
use simdnbt::borrow::NbtCompound as NbtCompoundView;
use simdnbt::owned::NbtCompound;
use steel_registry::blocks::block_state_ext::BlockStateExt as _;
use steel_registry::blocks::properties::{BlockStateProperties, BoolProperty};
use steel_registry::vanilla_block_entity_types;
use steel_utils::{
    BlockPos, BlockStateId, DowncastType, DowncastTypeKey, locks::SyncMutex, nbt::NbtValueInput as _,
    translations,
};
use text_components::TextComponent;

use crate::{
    block_entity::{
        BlockEntity, BlockEntityBase, ImplicitComponentGetter,
        container_properties::ContainerProperties,
        randomizable_container::RandomizableContainerStorage,
    },
    inventory::{
        lock::{ContainerLockGuard, ContainerRef},
        transfer::move_one_item,
    },
    nameable::Nameable,
    world::World,
};

/// Number of slots in a hopper.
pub const HOPPER_SLOTS: usize = 5;

/// Vanilla `HopperBlockEntity.MOVE_ITEM_SPEED`.
pub const MOVE_ITEM_SPEED: i32 = 8;

/// Extra NBT key used to persist the transfer cooldown.
const COOLDOWN_NBT_KEY: &str = "TransferCooldown";

/// Hopper inventory and transfer state.
pub struct HopperBlockEntity {
    storage: RandomizableContainerStorage,
    properties: ContainerProperties,
    cooldown: SyncMutex<i32>,
}

// SAFETY: This Steel-owned key uniquely identifies `HopperBlockEntity`.
unsafe impl DowncastType for HopperBlockEntity {
    const TYPE_KEY: DowncastTypeKey = DowncastTypeKey::new("steel:block_entity/hopper");
}

impl HopperBlockEntity {
    /// Creates an empty hopper.
    #[must_use]
    pub fn new(level: Weak<World>, pos: BlockPos, state: BlockStateId) -> Self {
        Self {
            storage: RandomizableContainerStorage::new(
                &vanilla_block_entity_types::HOPPER,
                HOPPER_SLOTS,
                level,
                pos,
                state,
            ),
            properties: ContainerProperties::default(),
            cooldown: SyncMutex::new(0),
        }
    }

    /// Returns the current transfer cooldown in game ticks.
    #[must_use]
    pub fn cooldown(&self) -> i32 {
        *self.cooldown.lock()
    }

    /// Advances one game tick of hopper logic.
    fn server_tick(&self, world: &Arc<World>) {
        if self.base().is_removed() {
            return;
        }
        let mut cooldown = self.cooldown.lock();
        if *cooldown > 0 {
            *cooldown -= 1;
            // The tick that brings the cooldown to zero performs the next transfer,
            // keeping the Vanilla 8-game-tick cadence (2.5 items per second).
            if *cooldown > 0 {
                return;
            }
        }
        let state = self.get_block_state();
        if !state.get_value(ENABLED) {
            return;
        }
        *cooldown = MOVE_ITEM_SPEED;
        drop(cooldown);

        let facing = state.get_value(FACING);
        self.push_one(world, facing);
        if facing != steel_registry::blocks::properties::Direction::Up {
            self.pull_one(world);
        }
    }

    /// Moves at most one item into the container this hopper faces.
    fn push_one(&self, world: &Arc<World>, facing: steel_registry::blocks::properties::Direction) -> bool {
        let target_pos = self.get_block_pos().relative(facing);
        let Some(target_entity) = world.get_block_entity(target_pos) else {
            return false;
        };
        let Some(target) = target_entity.container_ref() else {
            return false;
        };
        let own = self.storage.container_ref.clone();
        let Ok(mut guard) = ContainerLockGuard::lock_all(&[&own, &target]) else {
            log::warn!(
                "could not lock hopper push containers at {:?}",
                self.get_block_pos()
            );
            return false;
        };
        move_one_item(&mut guard, own.container_id(), target.container_id())
    }

    /// Moves at most one item out of the container above this hopper.
    fn pull_one(&self, world: &Arc<World>) -> bool {
        let source_pos = self.get_block_pos().above();
        let Some(source_entity) = world.get_block_entity(source_pos) else {
            return false;
        };
        let Some(source) = source_entity.container_ref() else {
            return false;
        };
        let own = self.storage.container_ref.clone();
        let Ok(mut guard) = ContainerLockGuard::lock_all(&[&source, &own]) else {
            log::warn!("could not lock hopper pull containers at {:?}", self.get_block_pos());
            return false;
        };
        move_one_item(&mut guard, source.container_id(), own.container_id())
    }
}

impl BlockEntity for HopperBlockEntity {
    fn base(&self) -> &BlockEntityBase {
        &self.storage.base
    }

    fn load_additional(&self, nbt: &BorrowedNbtCompound<'_>) {
        self.properties.load(nbt);
        self.storage.load_additional(nbt);
        let view: NbtCompoundView<'_, '_> = nbt.into();
        *self.cooldown.lock() = view.int_value(COOLDOWN_NBT_KEY).unwrap_or(0).clamp(0, MOVE_ITEM_SPEED);
    }

    fn save_additional(&self, nbt: &mut NbtCompound) {
        self.properties.save(nbt);
        self.storage.save_additional(nbt);
        nbt.insert(COOLDOWN_NBT_KEY, self.cooldown());
    }

    fn pre_remove_side_effects(&self, pos: BlockPos, _state: BlockStateId) {
        self.storage.pre_remove_side_effects(pos);
    }

    fn apply_implicit_components(&self, components: &mut ImplicitComponentGetter<'_>) {
        self.properties.apply_components(components);
        self.storage.apply_implicit_components(components);
    }

    fn container_ref(&self) -> Option<ContainerRef> {
        Some(self.storage.container_ref.clone())
    }

    fn as_nameable(&self) -> Option<&dyn Nameable> {
        Some(self)
    }

    fn tick(&self, world: &Arc<World>) {
        self.server_tick(world);
    }
}

impl Nameable for HopperBlockEntity {
    fn custom_name(&self) -> Option<TextComponent> {
        self.properties.custom_name()
    }

    fn name(&self) -> TextComponent {
        self.properties
            .custom_name()
            .unwrap_or_else(|| translations::CONTAINER_HOPPER.msg().component())
    }
}

const ENABLED: &BoolProperty = &BlockStateProperties::ENABLED;
const FACING: &steel_registry::blocks::properties::EnumProperty<
    steel_registry::blocks::properties::Direction,
> = &BlockStateProperties::FACING_HOPPER;

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use steel_registry::{item_stack::ItemStack, vanilla_items};
    use steel_utils::locks::{IntoShared as _, SyncMutex};

    use super::HOPPER_SLOTS;
    use crate::inventory::{
        container::{Container, SimpleContainer},
        lock::{ContainerLockGuard, ContainerRef},
        transfer::move_one_item,
    };

    fn container_ref(size: usize) -> (Arc<SyncMutex<SimpleContainer>>, ContainerRef) {
        let container = SimpleContainer::new(size).into_shared();
        let reference = ContainerRef::from(&container);
        (container, reference)
    }

    #[test]
    fn moves_exactly_one_item_per_call() {
        let (source_container, source) = container_ref(HOPPER_SLOTS);
        let (target_container, target) = container_ref(6);
        source_container
            .lock()
            .set_item(0, ItemStack::with_count(&vanilla_items::STONE, 5));

        let mut guard = ContainerLockGuard::lock_all(&[&source, &target]).expect("locks");
        assert!(move_one_item(
            &mut guard,
            source.container_id(),
            target.container_id()
        ));
        drop(guard);

        assert_eq!(source_container.lock().get_item(0).count(), 4);
        assert_eq!(target_container.lock().get_item(0).count(), 1);

        let mut guard = ContainerLockGuard::lock_all(&[&source, &target]).expect("locks");
        assert!(move_one_item(
            &mut guard,
            source.container_id(),
            target.container_id()
        ));
        drop(guard);

        assert_eq!(source_container.lock().get_item(0).count(), 3);
        assert_eq!(target_container.lock().get_item(0).count(), 2);
    }

    #[test]
    fn respects_max_stack_size() {
        let (source_container, source) = container_ref(HOPPER_SLOTS);
        let (target_container, target) = container_ref(1);
        source_container
            .lock()
            .set_item(0, ItemStack::with_count(&vanilla_items::STONE, 3));
        target_container
            .lock()
            .set_item(0, ItemStack::with_count(&vanilla_items::STONE, 64));

        let mut guard = ContainerLockGuard::lock_all(&[&source, &target]).expect("locks");
        assert!(!move_one_item(
            &mut guard,
            source.container_id(),
            target.container_id()
        ));
        drop(guard);

        assert_eq!(source_container.lock().get_item(0).count(), 3);
        assert_eq!(target_container.lock().get_item(0).count(), 64);
    }

    #[test]
    fn fills_empty_slots_before_stacking() {
        let (source_container, source) = container_ref(HOPPER_SLOTS);
        let (target_container, target) = container_ref(2);
        source_container
            .lock()
            .set_item(0, ItemStack::with_count(&vanilla_items::STONE, 2));
        target_container
            .lock()
            .set_item(0, ItemStack::with_count(&vanilla_items::DIRT, 1));

        let mut guard = ContainerLockGuard::lock_all(&[&source, &target]).expect("locks");
        assert!(move_one_item(
            &mut guard,
            source.container_id(),
            target.container_id()
        ));
        drop(guard);

        assert_eq!(target_container.lock().get_item(0).count(), 1);
        assert_eq!(target_container.lock().get_item(1).count(), 1);
    }
}
