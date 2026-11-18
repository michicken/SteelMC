//! Vanilla boat entity (all wood variants share this class).

use std::sync::{Arc, Weak};

use glam::DVec3;

use steel_macros::entity_behavior;
use steel_registry::entity_type::EntityTypeRef;
use steel_utils::{DowncastType, DowncastTypeKey};

use crate::entity::{Entity, EntityBase, EntityBaseLoad, SharedEntity};
use crate::world::World;

/// A rideable boat. Wood variants are distinguished by `entity_type`; the
/// vanilla model/logic is shared through the `Boat` class.
#[entity_behavior(class = "Boat")]
pub struct BoatEntity {
    base: EntityBase,
    entity_type: EntityTypeRef,
}

// SAFETY: This key is owned by Steel and uniquely identifies `BoatEntity`.
unsafe impl DowncastType for BoatEntity {
    const TYPE_KEY: DowncastTypeKey = DowncastTypeKey::new("steel:entity/boat");
}

impl BoatEntity {
    /// Creates a new boat entity.
    #[must_use]
    pub fn new(entity_type: EntityTypeRef, id: i32, position: DVec3, world: Weak<World>) -> Self {
        Self {
            base: EntityBase::new(id, position, entity_type.dimensions, world),
            entity_type,
        }
    }

    /// Creates a boat from saved data.
    #[must_use]
    pub fn from_saved(entity_type: EntityTypeRef, load: EntityBaseLoad) -> Self {
        Self {
            base: EntityBase::from_load(load, entity_type.dimensions),
            entity_type,
        }
    }
}

impl Entity for BoatEntity {
    fn base(&self) -> &EntityBase {
        &self.base
    }

    fn entity_type(&self) -> EntityTypeRef {
        self.entity_type
    }

    fn tick(self: Arc<Self>) {
        // Boats are non-living vehicles; the base tick keeps synced state and
        // passenger bookkeeping current. Water physics is handled separately
        // by the vehicle layer when present.
        self.base_tick();
    }

    fn can_add_passenger(&self, _passenger: &dyn Entity) -> bool {
        self.passengers().len() < 2
    }

    fn could_accept_passenger(&self) -> bool {
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::test_world;
    use steel_registry::vanilla_entities;

    #[test]
    fn boat_entity_accepts_passengers_and_registers_type() {
        let world = test_world();
        let boat = BoatEntity::new(
            &vanilla_entities::OAK_BOAT,
            0,
            DVec3::new(0.0, 64.0, 0.0),
            Arc::downgrade(world),
        );
        assert!(boat.could_accept_passenger());
        assert_eq!(boat.entity_type().key.path.as_ref(), "oak_boat");
    }
}
