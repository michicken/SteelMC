//! Vanilla armor stand entity.

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

/// An armor stand with the six vanilla equipment slots (main hand, off hand,
/// head, chest, legs, feet).
#[entity_behavior(class = "ArmorStand")]
pub struct ArmorStandEntity {
    base: EntityBase,
    entity_type: EntityTypeRef,
    equipment: OnceLock<Shared<SimpleContainer>>,
}

// SAFETY: This key is owned by Steel and uniquely identifies `ArmorStandEntity`.
unsafe impl DowncastType for ArmorStandEntity {
    const TYPE_KEY: DowncastTypeKey = DowncastTypeKey::new("steel:entity/armor_stand");
}

impl ArmorStandEntity {
    /// Creates a new armor stand.
    #[must_use]
    pub fn new(entity_type: EntityTypeRef, id: i32, position: DVec3, world: Weak<World>) -> Self {
        Self {
            base: EntityBase::new(id, position, entity_type.dimensions, world),
            entity_type,
            equipment: OnceLock::new(),
        }
    }

    /// The six equipment slots: main hand, off hand, head, chest, legs, feet.
    #[must_use]
    pub fn equipment_container(&self) -> ContainerRef {
        let equipment = self
            .equipment
            .get_or_init(|| Arc::new(SyncMutex::new(SimpleContainer::new(6))));
        ContainerRef::from(equipment)
    }

    /// Creates an armor stand from saved data.
    #[must_use]
    pub fn from_saved(entity_type: EntityTypeRef, load: EntityBaseLoad) -> Self {
        Self {
            base: EntityBase::from_load(load, entity_type.dimensions),
            entity_type,
            equipment: OnceLock::new(),
        }
    }
}

impl Entity for ArmorStandEntity {
    fn base(&self) -> &EntityBase {
        &self.base
    }

    fn entity_type(&self) -> EntityTypeRef {
        self.entity_type
    }

    fn tick(self: Arc<Self>) {}

    fn no_physics(&self) -> bool {
        true
    }

    fn can_add_passenger(&self, _passenger: &dyn Entity) -> bool {
        false
    }

    fn could_accept_passenger(&self) -> bool {
        false
    }
}
