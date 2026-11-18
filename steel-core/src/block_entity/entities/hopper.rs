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

use glam::DVec3;
use simdnbt::borrow::BaseNbtCompound as BorrowedNbtCompound;
use simdnbt::borrow::NbtCompound as NbtCompoundView;
use simdnbt::owned::NbtCompound;
use steel_registry::blocks::block_state_ext::BlockStateExt as _;
use steel_registry::blocks::properties::{BlockStateProperties, BoolProperty};
use steel_registry::{item_stack::ItemStack, vanilla_block_entity_types, vanilla_blocks, vanilla_entities};
use steel_utils::{
    BlockPos, BlockStateId, Downcast as _, DowncastType, DowncastTypeKey, WorldAabb,
    locks::SyncMutex, nbt::NbtValueInput as _, translations,
};
use text_components::TextComponent;

use crate::{
    block_entity::{
        BlockEntity, BlockEntityBase, ImplicitComponentGetter,
        container_properties::ContainerProperties,
        randomizable_container::RandomizableContainerStorage,
    },
    entity::{Entity as _, RemovalReason, entities::ItemEntity},
    inventory::{
        lock::{ContainerLockGuard, ContainerRef},
        transfer::{insert_stack, move_one_item_in_slots},
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
        let state = self.get_block_state();
        if !state.get_value(ENABLED) {
            return;
        }
        // Vanilla hoppers suck items resting above them every tick, independent of
        // the container-to-container transfer cooldown.
        self.suck_item_entities(world);

        let mut cooldown = self.cooldown.lock();
        if *cooldown > 0 {
            *cooldown -= 1;
            // The tick that brings the cooldown to zero performs the next transfer,
            // keeping the Vanilla 8-game-tick cadence (2.5 items per second).
            if *cooldown > 0 {
                return;
            }
        }
        *cooldown = MOVE_ITEM_SPEED;
        drop(cooldown);

        let facing = state.get_value(FACING);
        self.push_one(world, facing);
        if facing != steel_registry::blocks::properties::Direction::Up {
            self.pull_one(world);
        }
    }

    /// Pulls item entities resting in the Vanilla suction area above the hopper.
    ///
    /// The area is `(0.0, 0.6875, 0.0)..(1.0, 2.0, 1.0)` relative to the hopper, matching
    /// Vanilla's `HopperBlockEntity.SUCK_AREA`.
    fn suck_item_entities(&self, world: &Arc<World>) {
        let pos = self.get_block_pos();
        let center = DVec3::new(
            f64::from(pos.x()) + 0.5,
            f64::from(pos.y()) + 1.343_75,
            f64::from(pos.z()) + 0.5,
        );
        let area = WorldAabb::of_size(center, 1.0, 1.312_5, 1.0);
        let items = world.get_entities_in_aabb_matching(&area, |entity| {
            entity.entity_type() == &vanilla_entities::ITEM
        });
        for entity in items {
            let Some(item_entity) = entity.downcast_ref::<ItemEntity>() else {
                continue;
            };
            let stack = item_entity.get_item();
            if stack.is_empty() {
                continue;
            }
            let own = self.storage.container_ref.clone();
            let Ok(mut guard) = ContainerLockGuard::lock_all(&[&own]) else {
                continue;
            };
            let leftover = insert_stack(&mut guard, own.container_id(), stack.clone());
            drop(guard);
            if leftover.count() == stack.count() {
                continue;
            }
            if leftover.is_empty() {
                item_entity.set_removed(RemovalReason::Discarded);
            } else {
                item_entity.set_item(leftover);
            }
        }
    }

    /// Feeds one compostable item from this hopper into the composter it faces.
    fn push_into_composter(&self, world: &Arc<World>, target_pos: BlockPos) -> bool {
        let own = self.storage.container_ref.clone();
        let Ok(mut guard) = ContainerLockGuard::lock_all(&[&own]) else {
            return false;
        };
        let Some(stack) = guard.get(own.container_id()).map(|container| container.get_item(0).clone()) else {
            return false;
        };
        if !crate::behavior::blocks::building::is_compostable(&stack)
            || !crate::behavior::blocks::building::try_compost(world, target_pos, &stack)
        {
            return false;
        }
        let remaining = stack.count() - 1;
        let replacement = if remaining <= 0 {
            ItemStack::empty()
        } else {
            stack.copy_with_count(remaining)
        };
        if let Some(container) = guard.get_mut(own.container_id()) {
            container.set_item(0, replacement);
        }
        true
    }

    /// Pulls one bone meal out of a ready composter above this hopper.
    fn pull_from_composter(&self, world: &Arc<World>, source_pos: BlockPos) -> bool {
        let Some(bone_meal) =
            crate::behavior::blocks::building::harvest_bone_meal(world, source_pos)
        else {
            return false;
        };
        let own = self.storage.container_ref.clone();
        let Ok(mut guard) = ContainerLockGuard::lock_all(&[&own]) else {
            return false;
        };
        let leftover = insert_stack(&mut guard, own.container_id(), bone_meal);
        if leftover.is_empty() {
            true
        } else {
            // The hopper could not accept the output: restore the ready state.
            let state = world.get_block_state(source_pos);
            world.set_block(
                source_pos,
                state.set_value(&steel_registry::blocks::properties::BlockStateProperties::LEVEL_COMPOSTER, 8),
                steel_utils::types::UpdateFlags::UPDATE_ALL,
            );
            false
        }
    }

    /// Moves at most one item into the container this hopper faces.
    fn push_one(&self, world: &Arc<World>, facing: steel_registry::blocks::properties::Direction) -> bool {
        let target_pos = self.get_block_pos().relative(facing);
        if world.get_block_state(target_pos).get_block() == &vanilla_blocks::COMPOSTER {
            return self.push_into_composter(world, target_pos);
        }
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
        // Insert through the target's face opposite the hopper's output side
        // (a hopper facing down inserts into a furnace's `Up` view).
        let target_slots = guard
            .get(target.container_id())
            .and_then(|container| container.slots_for_face(facing.opposite()))
            .map(<[usize]>::to_vec);
        move_one_item_in_slots(
            &mut guard,
            own.container_id(),
            target.container_id(),
            None,
            target_slots.as_deref(),
            None,
        )
    }

    /// Moves at most one item out of the container above this hopper.
    fn pull_one(&self, world: &Arc<World>) -> bool {
        let source_pos = self.get_block_pos().above();
        if world.get_block_state(source_pos).get_block() == &vanilla_blocks::COMPOSTER {
            return self.pull_from_composter(world, source_pos);
        }
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
        // A hopper pulls through the source container's `Down` automation view:
        // a furnace below-face view exposes only the result slot, so raw input
        // and fuel can never be sucked out by the hopper underneath.
        let source_slots = guard
            .get(source.container_id())
            .and_then(|container| container.slots_for_face(steel_registry::blocks::properties::Direction::Down))
            .map(<[usize]>::to_vec);
        move_one_item_in_slots(
            &mut guard,
            source.container_id(),
            own.container_id(),
            source_slots.as_deref(),
            None,
            Some(steel_registry::blocks::properties::Direction::Down),
        )
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

    #[test]
    fn hopper_feeds_composter_and_collects_bone_meal() {
        use steel_registry::{init_vanilla_registry, vanilla_blocks};
        use steel_utils::{BlockPos, ChunkPos, types::UpdateFlags};

        use crate::behavior::init_behaviors;
        use crate::block_entity::init_block_entities;
        use crate::test_support::{fresh_test_world, insert_ready_full_chunk};

        init_vanilla_registry();
        init_behaviors();
        init_block_entities();
        let fixture = fresh_test_world("hopper_composter");
        let world = &fixture.world;
        insert_ready_full_chunk(world, ChunkPos::new(0, 0));

        let top_pos = BlockPos::new(8, 65, 8);
        let composter_pos = BlockPos::new(8, 64, 8);
        let bottom_pos = BlockPos::new(8, 63, 8);
        let chest_pos = BlockPos::new(8, 62, 8);
        world.set_block(top_pos, vanilla_blocks::HOPPER.default_state(), UpdateFlags::UPDATE_ALL);
        world.set_block(composter_pos, vanilla_blocks::COMPOSTER.default_state(), UpdateFlags::UPDATE_ALL);
        world.set_block(bottom_pos, vanilla_blocks::HOPPER.default_state(), UpdateFlags::UPDATE_ALL);
        world.set_block(chest_pos, vanilla_blocks::CHEST.default_state(), UpdateFlags::UPDATE_ALL);

        let top = world.get_block_entity(top_pos).expect("top hopper");
        let top_ref = ContainerRef::from_block_entity(top).expect("top container");
        {
            let mut guard = ContainerLockGuard::lock_all(&[&top_ref]).expect("lock top");
            guard
                .get_mut(top_ref.container_id())
                .expect("top inventory")
                .set_item(0, ItemStack::with_count(&vanilla_items::OAK_SAPLING, 8));
        }

        for _ in 0..120 {
            if let Some(hopper) = world.get_block_entity(top_pos) {
                hopper.tick(world);
            }
            if let Some(hopper) = world.get_block_entity(bottom_pos) {
                hopper.tick(world);
            }
        }

        let chest = world.get_block_entity(chest_pos).expect("chest");
        let chest_ref = ContainerRef::from_block_entity(chest).expect("chest container");
        let guard = ContainerLockGuard::lock_all(&[&chest_ref]).expect("lock chest");
        let inventory = guard.get(chest_ref.container_id()).expect("chest inventory");
        assert!(
            (0..27).any(|slot| inventory.get_item(slot).is(&vanilla_items::BONE_MEAL)),
            "hopper chain must turn saplings into bone meal in the chest"
        );
    }
}
