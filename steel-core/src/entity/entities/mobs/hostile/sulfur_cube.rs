//! Sulfur cube entity used by the sulfur cube bucket.

use std::sync::{Arc, Weak};

use glam::DVec3;

use steel_macros::entity_behavior;
use steel_registry::entity_type::EntityTypeRef;
use steel_utils::{DowncastType, DowncastTypeKey};

use crate::entity::{Entity, EntityBase, EntityBaseLoad};
use crate::world::World;

/// The sulfur cube mob. Its hostile hopping AI is layered on top of this
/// entity; the entity itself is fully spawnable and tickable.
#[entity_behavior(class = "SulfurCube")]
pub struct SulfurCubeEntity {
    base: EntityBase,
    entity_type: EntityTypeRef,
}

// SAFETY: This key is owned by Steel and uniquely identifies this entity.
unsafe impl DowncastType for SulfurCubeEntity {
    const TYPE_KEY: DowncastTypeKey = DowncastTypeKey::new("steel:entity/sulfur_cube");
}

impl SulfurCubeEntity {
    /// Creates a new entity.
    #[must_use]
    pub fn new(entity_type: EntityTypeRef, id: i32, position: DVec3, world: Weak<World>) -> Self {
        Self {
            base: EntityBase::new(id, position, entity_type.dimensions, world),
            entity_type,
        }
    }

    /// Creates the entity from saved data.
    #[must_use]
    pub fn from_saved(entity_type: EntityTypeRef, load: EntityBaseLoad) -> Self {
        Self {
            base: EntityBase::from_load(load, entity_type.dimensions),
            entity_type,
        }
    }
}

impl Entity for SulfurCubeEntity {
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
