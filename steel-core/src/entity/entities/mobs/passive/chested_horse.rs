//! Donkey/mule/llama chest carriers used by the chest dispenser branch.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock, Weak};

use glam::DVec3;

use steel_macros::entity_behavior;
use steel_registry::entity_type::EntityTypeRef;
use steel_utils::locks::{Shared, SyncMutex};
use steel_utils::{DowncastType, DowncastTypeKey};

use crate::entity::{Entity, EntityBase, EntityBaseLoad};
use crate::inventory::container::SimpleContainer;
use crate::inventory::lock::ContainerRef;
use crate::world::World;

/// `Donkey` that can carry a chest.
#[entity_behavior(class = "Donkey")]
pub struct DonkeyEntity {
    base: EntityBase,
    entity_type: EntityTypeRef,
    has_chest: AtomicBool,
    chest: OnceLock<Shared<SimpleContainer>>,
}

// SAFETY: This key is owned by Steel and uniquely identifies this entity.
unsafe impl DowncastType for DonkeyEntity {
    const TYPE_KEY: DowncastTypeKey = DowncastTypeKey::new("steel:entity/donkey");
}

impl DonkeyEntity {
    /// Creates a new entity.
    #[must_use]
    pub fn new(entity_type: EntityTypeRef, id: i32, position: DVec3, world: Weak<World>) -> Self {
        Self {
            base: EntityBase::new(id, position, entity_type.dimensions, world),
            entity_type,
            has_chest: AtomicBool::new(false),
            chest: OnceLock::new(),
        }
    }

    /// Creates the entity from saved data.
    #[must_use]
    pub fn from_saved(entity_type: EntityTypeRef, load: EntityBaseLoad) -> Self {
        Self {
            base: EntityBase::from_load(load, entity_type.dimensions),
            entity_type,
            has_chest: AtomicBool::new(false),
            chest: OnceLock::new(),
        }
    }

    /// Equips a chest. Returns false when a chest is already present.
    pub fn equip_chest(&self) -> bool {
        if self.has_chest.swap(true, Ordering::SeqCst) {
            return false;
        }
        let _ = self.chest.set(Arc::new(SyncMutex::new(SimpleContainer::new(15))));
        true
    }

    /// Whether this carrier already has a chest.
    #[must_use]
    pub fn has_chest(&self) -> bool {
        self.has_chest.load(Ordering::SeqCst)
    }

    /// The chest inventory, once equipped.
    #[must_use]
    pub fn chest_container(&self) -> Option<ContainerRef> {
        self.chest.get().map(ContainerRef::from)
    }
}

impl Entity for DonkeyEntity {
    fn base(&self) -> &EntityBase {
        &self.base
    }

    fn entity_type(&self) -> EntityTypeRef {
        self.entity_type
    }

    fn tick(self: Arc<Self>) {
        self.base_tick();
    }

    fn can_add_passenger(&self, _passenger: &dyn Entity) -> bool {
        false
    }

    fn could_accept_passenger(&self) -> bool {
        false
    }
}

/// `Mule` that can carry a chest.
#[entity_behavior(class = "Mule")]
pub struct MuleEntity {
    base: EntityBase,
    entity_type: EntityTypeRef,
    has_chest: AtomicBool,
    chest: OnceLock<Shared<SimpleContainer>>,
}

// SAFETY: This key is owned by Steel and uniquely identifies this entity.
unsafe impl DowncastType for MuleEntity {
    const TYPE_KEY: DowncastTypeKey = DowncastTypeKey::new("steel:entity/mule");
}

impl MuleEntity {
    /// Creates a new entity.
    #[must_use]
    pub fn new(entity_type: EntityTypeRef, id: i32, position: DVec3, world: Weak<World>) -> Self {
        Self {
            base: EntityBase::new(id, position, entity_type.dimensions, world),
            entity_type,
            has_chest: AtomicBool::new(false),
            chest: OnceLock::new(),
        }
    }

    /// Creates the entity from saved data.
    #[must_use]
    pub fn from_saved(entity_type: EntityTypeRef, load: EntityBaseLoad) -> Self {
        Self {
            base: EntityBase::from_load(load, entity_type.dimensions),
            entity_type,
            has_chest: AtomicBool::new(false),
            chest: OnceLock::new(),
        }
    }

    /// Equips a chest. Returns false when a chest is already present.
    pub fn equip_chest(&self) -> bool {
        if self.has_chest.swap(true, Ordering::SeqCst) {
            return false;
        }
        let _ = self.chest.set(Arc::new(SyncMutex::new(SimpleContainer::new(15))));
        true
    }

    /// Whether this carrier already has a chest.
    #[must_use]
    pub fn has_chest(&self) -> bool {
        self.has_chest.load(Ordering::SeqCst)
    }

    /// The chest inventory, once equipped.
    #[must_use]
    pub fn chest_container(&self) -> Option<ContainerRef> {
        self.chest.get().map(ContainerRef::from)
    }
}

impl Entity for MuleEntity {
    fn base(&self) -> &EntityBase {
        &self.base
    }

    fn entity_type(&self) -> EntityTypeRef {
        self.entity_type
    }

    fn tick(self: Arc<Self>) {
        self.base_tick();
    }

    fn can_add_passenger(&self, _passenger: &dyn Entity) -> bool {
        false
    }

    fn could_accept_passenger(&self) -> bool {
        false
    }
}

/// `Llama` that can carry a chest.
#[entity_behavior(class = "Llama")]
pub struct LlamaEntity {
    base: EntityBase,
    entity_type: EntityTypeRef,
    has_chest: AtomicBool,
    chest: OnceLock<Shared<SimpleContainer>>,
}

// SAFETY: This key is owned by Steel and uniquely identifies this entity.
unsafe impl DowncastType for LlamaEntity {
    const TYPE_KEY: DowncastTypeKey = DowncastTypeKey::new("steel:entity/llama");
}

impl LlamaEntity {
    /// Creates a new entity.
    #[must_use]
    pub fn new(entity_type: EntityTypeRef, id: i32, position: DVec3, world: Weak<World>) -> Self {
        Self {
            base: EntityBase::new(id, position, entity_type.dimensions, world),
            entity_type,
            has_chest: AtomicBool::new(false),
            chest: OnceLock::new(),
        }
    }

    /// Creates the entity from saved data.
    #[must_use]
    pub fn from_saved(entity_type: EntityTypeRef, load: EntityBaseLoad) -> Self {
        Self {
            base: EntityBase::from_load(load, entity_type.dimensions),
            entity_type,
            has_chest: AtomicBool::new(false),
            chest: OnceLock::new(),
        }
    }

    /// Equips a chest. Returns false when a chest is already present.
    pub fn equip_chest(&self) -> bool {
        if self.has_chest.swap(true, Ordering::SeqCst) {
            return false;
        }
        let _ = self.chest.set(Arc::new(SyncMutex::new(SimpleContainer::new(3))));
        true
    }

    /// Whether this carrier already has a chest.
    #[must_use]
    pub fn has_chest(&self) -> bool {
        self.has_chest.load(Ordering::SeqCst)
    }

    /// The chest inventory, once equipped.
    #[must_use]
    pub fn chest_container(&self) -> Option<ContainerRef> {
        self.chest.get().map(ContainerRef::from)
    }
}

impl Entity for LlamaEntity {
    fn base(&self) -> &EntityBase {
        &self.base
    }

    fn entity_type(&self) -> EntityTypeRef {
        self.entity_type
    }

    fn tick(self: Arc<Self>) {
        self.base_tick();
    }

    fn can_add_passenger(&self, _passenger: &dyn Entity) -> bool {
        false
    }

    fn could_accept_passenger(&self) -> bool {
        false
    }
}

/// `TraderLlama` that can carry a chest.
#[entity_behavior(class = "TraderLlama")]
pub struct TraderLlamaEntity {
    base: EntityBase,
    entity_type: EntityTypeRef,
    has_chest: AtomicBool,
    chest: OnceLock<Shared<SimpleContainer>>,
}

// SAFETY: This key is owned by Steel and uniquely identifies this entity.
unsafe impl DowncastType for TraderLlamaEntity {
    const TYPE_KEY: DowncastTypeKey = DowncastTypeKey::new("steel:entity/trader_llama");
}

impl TraderLlamaEntity {
    /// Creates a new entity.
    #[must_use]
    pub fn new(entity_type: EntityTypeRef, id: i32, position: DVec3, world: Weak<World>) -> Self {
        Self {
            base: EntityBase::new(id, position, entity_type.dimensions, world),
            entity_type,
            has_chest: AtomicBool::new(false),
            chest: OnceLock::new(),
        }
    }

    /// Creates the entity from saved data.
    #[must_use]
    pub fn from_saved(entity_type: EntityTypeRef, load: EntityBaseLoad) -> Self {
        Self {
            base: EntityBase::from_load(load, entity_type.dimensions),
            entity_type,
            has_chest: AtomicBool::new(false),
            chest: OnceLock::new(),
        }
    }

    /// Equips a chest. Returns false when a chest is already present.
    pub fn equip_chest(&self) -> bool {
        if self.has_chest.swap(true, Ordering::SeqCst) {
            return false;
        }
        let _ = self.chest.set(Arc::new(SyncMutex::new(SimpleContainer::new(3))));
        true
    }

    /// Whether this carrier already has a chest.
    #[must_use]
    pub fn has_chest(&self) -> bool {
        self.has_chest.load(Ordering::SeqCst)
    }

    /// The chest inventory, once equipped.
    #[must_use]
    pub fn chest_container(&self) -> Option<ContainerRef> {
        self.chest.get().map(ContainerRef::from)
    }
}

impl Entity for TraderLlamaEntity {
    fn base(&self) -> &EntityBase {
        &self.base
    }

    fn entity_type(&self) -> EntityTypeRef {
        self.entity_type
    }

    fn tick(self: Arc<Self>) {
        self.base_tick();
    }

    fn can_add_passenger(&self, _passenger: &dyn Entity) -> bool {
        false
    }

    fn could_accept_passenger(&self) -> bool {
        false
    }
}

