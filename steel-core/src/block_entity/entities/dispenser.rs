//! Dispenser and dropper block entity.
//!
//! Both blocks share the same nine-slot container; only the dispense behavior of the
//! owning block differs.

use std::sync::{Arc, Weak};

use simdnbt::borrow::BaseNbtCompound as BorrowedNbtCompound;
use simdnbt::owned::NbtCompound;
use steel_registry::block_entity_type::BlockEntityTypeRef;
use steel_registry::vanilla_block_entity_types;
use steel_utils::{BlockPos, BlockStateId, DowncastType, DowncastTypeKey};

use crate::block_entity::{
    BlockEntity, BlockEntityBase, ImplicitComponentGetter,
    randomizable_container::RandomizableContainerStorage,
};
use crate::inventory::lock::ContainerRef;
use crate::world::World;

/// Number of slots in a dispenser or dropper.
pub const DISPENSER_SLOTS: usize = 9;

/// Shared inventory for dispensers and droppers.
pub struct DispenserBlockEntity {
    storage: RandomizableContainerStorage,
}

unsafe impl DowncastType for DispenserBlockEntity {
    const TYPE_KEY: DowncastTypeKey = DowncastTypeKey::new("steel:block_entity/dispenser");
}

impl DispenserBlockEntity {
    /// Creates a dispenser inventory.
    #[must_use]
    pub fn new_dispenser(level: Weak<World>, pos: BlockPos, state: BlockStateId) -> Self {
        Self::new(&vanilla_block_entity_types::DISPENSER, level, pos, state)
    }

    /// Creates a dropper inventory.
    #[must_use]
    pub fn new_dropper(level: Weak<World>, pos: BlockPos, state: BlockStateId) -> Self {
        Self::new(&vanilla_block_entity_types::DROPPER, level, pos, state)
    }

    fn new(
        entity_type: BlockEntityTypeRef,
        level: Weak<World>,
        pos: BlockPos,
        state: BlockStateId,
    ) -> Self {
        Self {
            storage: RandomizableContainerStorage::new(
                entity_type,
                DISPENSER_SLOTS,
                level,
                pos,
                state,
            ),
        }
    }
}

impl BlockEntity for DispenserBlockEntity {
    fn base(&self) -> &BlockEntityBase {
        &self.storage.base
    }

    fn load_additional(&self, nbt: &BorrowedNbtCompound<'_>) {
        self.storage.load_additional(nbt);
    }

    fn save_additional(&self, nbt: &mut NbtCompound) {
        self.storage.save_additional(nbt);
    }

    fn pre_remove_side_effects(&self, pos: BlockPos, _state: BlockStateId) {
        self.storage.pre_remove_side_effects(pos);
    }

    fn apply_implicit_components(&self, components: &mut ImplicitComponentGetter<'_>) {
        self.storage.apply_implicit_components(components);
    }

    fn container_ref(&self) -> Option<ContainerRef> {
        Some(self.storage.container_ref.clone())
    }

    fn tick(&self, _world: &Arc<World>) {}
}
