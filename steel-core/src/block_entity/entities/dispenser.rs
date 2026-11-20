//! Dispenser and dropper block entity.
//!
//! Both blocks share the same nine-slot container; only the dispense behavior of the
//! owning block differs.

use std::sync::{Arc, Weak};

use simdnbt::borrow::BaseNbtCompound as BorrowedNbtCompound;
use simdnbt::owned::NbtCompound;
use steel_registry::block_entity_type::BlockEntityTypeRef;
use steel_registry::data_components::vanilla_components::SeededContainerLoot;
use steel_registry::vanilla_block_entity_types;
use steel_utils::translations;
use steel_utils::{BlockPos, BlockStateId, DowncastType, DowncastTypeKey};
use text_components::TextComponent;

use crate::block_entity::{
    BlockEntity, BlockEntityBase, ImplicitComponentGetter, RandomizableContainer,
    container_properties::ContainerProperties,
    randomizable_container::RandomizableContainerStorage,
};
use crate::entity::Entity as _;
use crate::inventory::lock::ContainerRef;
use crate::loot::LootResult;
use crate::nameable::Nameable;
use crate::player::Player;
use crate::world::World;

/// Number of slots in a dispenser or dropper.
pub const DISPENSER_SLOTS: usize = 9;

/// Shared inventory for dispensers and droppers.
pub struct DispenserBlockEntity {
    storage: RandomizableContainerStorage,
    properties: ContainerProperties,
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
            properties: ContainerProperties::default(),
        }
    }

    pub(crate) fn can_open(&self, player: &Player) -> bool {
        (!player.is_spectator() || self.loot_table().is_none()) && self.properties.can_open(player)
    }
}

impl BlockEntity for DispenserBlockEntity {
    fn base(&self) -> &BlockEntityBase {
        &self.storage.base
    }

    fn load_additional(&self, nbt: &BorrowedNbtCompound<'_>) {
        self.properties.load(nbt);
        self.storage.load_additional(nbt);
    }

    fn save_additional(&self, nbt: &mut NbtCompound) {
        self.properties.save(nbt);
        self.storage.save_additional(nbt);
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

    fn as_randomizable_container(&self) -> Option<&dyn RandomizableContainer> {
        Some(self)
    }

    fn tick(&self, _world: &Arc<World>) {}
}

impl RandomizableContainer for DispenserBlockEntity {
    fn loot_table(&self) -> Option<SeededContainerLoot> {
        self.storage.loot_table()
    }

    fn set_loot_table(&self, loot: Option<SeededContainerLoot>) {
        self.storage.set_loot_table(loot);
    }

    fn unpack_loot_table(&self, player: Option<&Player>) -> LootResult<()> {
        self.storage.unpack_loot_table(player)
    }
}

impl Nameable for DispenserBlockEntity {
    fn custom_name(&self) -> Option<TextComponent> {
        self.properties.custom_name()
    }

    fn name(&self) -> TextComponent {
        self.custom_name().unwrap_or_else(|| {
            if self.get_type() == &vanilla_block_entity_types::DROPPER {
                translations::CONTAINER_DROPPER.msg().component()
            } else {
                translations::CONTAINER_DISPENSER.msg().component()
            }
        })
    }
}
