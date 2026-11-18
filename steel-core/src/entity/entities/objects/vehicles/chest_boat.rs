//! Vanilla chest boat entity.

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

/// A boat with a 27-slot chest inventory.
#[entity_behavior(class = "ChestBoat")]
pub struct ChestBoatEntity {
    base: EntityBase,
    entity_type: EntityTypeRef,
    chest: OnceLock<Shared<SimpleContainer>>,
}

// SAFETY: This key is owned by Steel and uniquely identifies `ChestBoatEntity`.
unsafe impl DowncastType for ChestBoatEntity {
    const TYPE_KEY: DowncastTypeKey = DowncastTypeKey::new("steel:entity/chest_boat");
}

impl ChestBoatEntity {
    /// Creates a new chest boat.
    #[must_use]
    pub fn new(entity_type: EntityTypeRef, id: i32, position: DVec3, world: Weak<World>) -> Self {
        Self {
            base: EntityBase::new(id, position, entity_type.dimensions, world),
            entity_type,
            chest: OnceLock::new(),
        }
    }

    /// The 27-slot chest inventory of this chest boat.
    #[must_use]
    pub fn chest_container(&self) -> ContainerRef {
        let chest = self
            .chest
            .get_or_init(|| Arc::new(SyncMutex::new(SimpleContainer::new(27))));
        ContainerRef::from(chest)
    }

    /// Creates a chest boat from saved data.
    #[must_use]
    pub fn from_saved(entity_type: EntityTypeRef, load: EntityBaseLoad) -> Self {
        Self {
            base: EntityBase::from_load(load, entity_type.dimensions),
            entity_type,
            chest: OnceLock::new(),
        }
    }
}

impl Entity for ChestBoatEntity {
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
        self.passengers().is_empty()
    }

    fn could_accept_passenger(&self) -> bool {
        true
    }
}
