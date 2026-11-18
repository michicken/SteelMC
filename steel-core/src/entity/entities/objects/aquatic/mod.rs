//! Simple aquatic mob entities used by mob buckets.

use std::sync::{Arc, Weak};

use glam::DVec3;

use steel_macros::entity_behavior;
use steel_registry::entity_type::EntityTypeRef;
use steel_utils::{DowncastType, DowncastTypeKey};

use crate::entity::{Entity, EntityBase, EntityBaseLoad};
use crate::world::World;

/// A real spawnable aquatic entity. Movement is supplied by the aquatic
/// AI layer when a bucket mob is released.
#[entity_behavior(class = "Axolotl")]
pub struct AxolotlEntity {
    base: EntityBase,
    entity_type: EntityTypeRef,
}

// SAFETY: This key is owned by Steel and uniquely identifies this entity.
unsafe impl DowncastType for AxolotlEntity {
    const TYPE_KEY: DowncastTypeKey = DowncastTypeKey::new("steel:entity/axolotl");
}

impl AxolotlEntity {
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

impl Entity for AxolotlEntity {
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

/// A real spawnable aquatic entity. Movement is supplied by the aquatic
/// AI layer when a bucket mob is released.
#[entity_behavior(class = "Cod")]
pub struct CodEntity {
    base: EntityBase,
    entity_type: EntityTypeRef,
}

// SAFETY: This key is owned by Steel and uniquely identifies this entity.
unsafe impl DowncastType for CodEntity {
    const TYPE_KEY: DowncastTypeKey = DowncastTypeKey::new("steel:entity/cod");
}

impl CodEntity {
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

impl Entity for CodEntity {
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

/// A real spawnable aquatic entity. Movement is supplied by the aquatic
/// AI layer when a bucket mob is released.
#[entity_behavior(class = "Salmon")]
pub struct SalmonEntity {
    base: EntityBase,
    entity_type: EntityTypeRef,
}

// SAFETY: This key is owned by Steel and uniquely identifies this entity.
unsafe impl DowncastType for SalmonEntity {
    const TYPE_KEY: DowncastTypeKey = DowncastTypeKey::new("steel:entity/salmon");
}

impl SalmonEntity {
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

impl Entity for SalmonEntity {
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

/// A real spawnable aquatic entity. Movement is supplied by the aquatic
/// AI layer when a bucket mob is released.
#[entity_behavior(class = "Pufferfish")]
pub struct PufferfishEntity {
    base: EntityBase,
    entity_type: EntityTypeRef,
}

// SAFETY: This key is owned by Steel and uniquely identifies this entity.
unsafe impl DowncastType for PufferfishEntity {
    const TYPE_KEY: DowncastTypeKey = DowncastTypeKey::new("steel:entity/pufferfish");
}

impl PufferfishEntity {
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

impl Entity for PufferfishEntity {
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

/// A real spawnable aquatic entity. Movement is supplied by the aquatic
/// AI layer when a bucket mob is released.
#[entity_behavior(class = "TropicalFish")]
pub struct TropicalFishEntity {
    base: EntityBase,
    entity_type: EntityTypeRef,
}

// SAFETY: This key is owned by Steel and uniquely identifies this entity.
unsafe impl DowncastType for TropicalFishEntity {
    const TYPE_KEY: DowncastTypeKey = DowncastTypeKey::new("steel:entity/tropical_fish");
}

impl TropicalFishEntity {
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

impl Entity for TropicalFishEntity {
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

/// A real spawnable aquatic entity. Movement is supplied by the aquatic
/// AI layer when a bucket mob is released.
#[entity_behavior(class = "Tadpole")]
pub struct TadpoleEntity {
    base: EntityBase,
    entity_type: EntityTypeRef,
}

// SAFETY: This key is owned by Steel and uniquely identifies this entity.
unsafe impl DowncastType for TadpoleEntity {
    const TYPE_KEY: DowncastTypeKey = DowncastTypeKey::new("steel:entity/tadpole");
}

impl TadpoleEntity {
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

impl Entity for TadpoleEntity {
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

