//! Armadillo entity used by brushing.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Weak};

use glam::DVec3;

use steel_macros::entity_behavior;
use steel_registry::entity_type::EntityTypeRef;
use steel_utils::{DowncastType, DowncastTypeKey};

use crate::entity::{Entity, EntityBase, EntityBaseLoad};
use crate::world::World;

/// The armadillo. Its curled/rolling AI is layered on top of this entity;
/// brushing state and scute drops live here.
#[entity_behavior(class = "Armadillo")]
pub struct ArmadilloEntity {
    base: EntityBase,
    entity_type: EntityTypeRef,
    brushed: AtomicBool,
}

// SAFETY: This key is owned by Steel and uniquely identifies this entity.
unsafe impl DowncastType for ArmadilloEntity {
    const TYPE_KEY: DowncastTypeKey = DowncastTypeKey::new("steel:entity/armadillo");
}

impl ArmadilloEntity {
    /// Creates a new entity.
    #[must_use]
    pub fn new(entity_type: EntityTypeRef, id: i32, position: DVec3, world: Weak<World>) -> Self {
        Self {
            base: EntityBase::new(id, position, entity_type.dimensions, world),
            entity_type,
            brushed: AtomicBool::new(false),
        }
    }

    /// Creates the entity from saved data.
    #[must_use]
    pub fn from_saved(entity_type: EntityTypeRef, load: EntityBaseLoad) -> Self {
        Self {
            base: EntityBase::from_load(load, entity_type.dimensions),
            entity_type,
            brushed: AtomicBool::new(false),
        }
    }

    /// Brushes this armadillo. Returns whether a scute is produced.
    pub fn brush(&self) -> bool {
        if self.brushed.swap(true, Ordering::SeqCst) {
            return false;
        }
        true
    }

    /// Restores the brushable state, as vanilla does after a delay.
    pub fn reset_brush(&self) {
        self.brushed.store(false, Ordering::SeqCst);
    }
}

impl Entity for ArmadilloEntity {
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
