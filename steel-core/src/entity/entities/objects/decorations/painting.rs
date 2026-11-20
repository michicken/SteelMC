//! Minimal vanilla painting entity.

use std::sync::Weak;

use glam::DVec3;
use steel_macros::entity_behavior;
use steel_registry::entity_type::EntityTypeRef;
use steel_registry::vanilla_entity_data::PaintingEntityData;
use steel_utils::locks::SyncMutex;
use steel_utils::{DowncastType, DowncastTypeKey, Direction};

use crate::entity::{Entity, EntityBase, EntityBaseLoad, EntitySyncedData};
use crate::world::World;

#[entity_behavior(class = "Painting")]
pub struct PaintingEntity {
    base: EntityBase,
    entity_type: EntityTypeRef,
    entity_data: SyncMutex<PaintingEntityData>,
}

unsafe impl DowncastType for PaintingEntity {
    const TYPE_KEY: DowncastTypeKey = DowncastTypeKey::new("steel:entity/painting");
}

impl PaintingEntity {
    /// Constructor used by the generated entity factory and `/summon`.
    #[must_use]
    pub fn new(
        entity_type: EntityTypeRef,
        id: i32,
        position: DVec3,
        world: Weak<World>,
    ) -> Self {
        Self {
            base: EntityBase::new(id, position, entity_type.dimensions, world),
            entity_type,
            entity_data: SyncMutex::new(PaintingEntityData::new()),
        }
    }

    /// Creates a painting placed against a wall, with the clicked face stored
    /// in its hanging-entity direction.
    #[must_use]
    pub fn new_with_direction(
        entity_type: EntityTypeRef,
        id: i32,
        position: DVec3,
        world: Weak<World>,
        direction: Direction,
    ) -> Self {
        let entity = Self::new(entity_type, id, position, world);
        entity.set_direction(direction);
        entity
    }

    /// Sets the wall facing used by the painting's placement and hitbox.
    pub fn set_direction(&self, direction: Direction) {
        self.entity_data
            .lock()
            .hanging_entity_mut()
            .direction
            .set(direction);
    }

    #[must_use]
    pub fn from_saved(entity_type: EntityTypeRef, load: EntityBaseLoad) -> Self {
        Self {
            base: EntityBase::from_load(load, entity_type.dimensions),
            entity_type,
            entity_data: SyncMutex::new(PaintingEntityData::new()),
        }
    }
}

impl Entity for PaintingEntity {
    fn base(&self) -> &EntityBase { &self.base }
    fn entity_type(&self) -> EntityTypeRef { self.entity_type }
    fn tick(self: std::sync::Arc<Self>) {}
    fn no_physics(&self) -> bool { true }
    fn synced_data(&self) -> Option<&dyn EntitySyncedData> { Some(&self.entity_data) }
}
