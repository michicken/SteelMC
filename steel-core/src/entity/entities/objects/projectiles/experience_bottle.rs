//! Thrown experience bottle projectile entity (`ExperienceBottle`).

use std::sync::Arc;
use std::sync::Weak;

use glam::DVec3;
use simdnbt::borrow::NbtCompound as BorrowedNbtCompoundView;
use simdnbt::owned::NbtCompound;
use steel_macros::entity_behavior;
use steel_protocol::packets::game::SoundSource;
use steel_registry::entity_type::EntityTypeRef;
use steel_registry::item_stack::ItemStack;
use steel_registry::items::ItemRef;
use steel_registry::vanilla_entity_data::ExperienceBottleEntityData;
use steel_registry::{vanilla_entities, vanilla_items};
use steel_utils::entity_events::EntityStatus;
use steel_utils::locks::SyncMutex;
use steel_utils::{DowncastType, DowncastTypeKey};

use crate::entity::{
    Entity, EntityBase, EntityBaseLoad, EntitySyncedData, Projectile, ProjectileBase, ProjectileHit,
    RemovalReason, SharedEntity, ThrowableItemProjectile, ThrowableProjectile,
    entities::ExperienceOrbEntity,
};
use crate::world::World;

/// A thrown bottle o' enchanting.
#[entity_behavior(class = "ThrownExperienceBottle")]
pub struct ExperienceBottleEntity {
    base: EntityBase,
    entity_type: EntityTypeRef,
    entity_data: SyncMutex<ExperienceBottleEntityData>,
    projectile_base: ProjectileBase,
}

// SAFETY: This key is owned by Steel and uniquely identifies `ExperienceBottleEntity`.
unsafe impl DowncastType for ExperienceBottleEntity {
    const TYPE_KEY: DowncastTypeKey = DowncastTypeKey::new("steel:entity/experience_bottle");
}

impl ExperienceBottleEntity {
    #[must_use]
    pub fn new(entity_type: EntityTypeRef, id: i32, position: DVec3, world: Weak<World>) -> Self {
        Self {
            base: EntityBase::new(id, position, entity_type.dimensions, world),
            entity_type,
            entity_data: SyncMutex::new(ExperienceBottleEntityData::new()),
            projectile_base: ProjectileBase::new(),
        }
    }

    #[must_use]
    pub fn from_saved(entity_type: EntityTypeRef, load: EntityBaseLoad) -> Self {
        Self {
            base: EntityBase::from_load(load, entity_type.dimensions),
            entity_type,
            entity_data: SyncMutex::new(ExperienceBottleEntityData::new()),
            projectile_base: ProjectileBase::new(),
        }
    }

    /// Vanilla `ThrownExperienceBottle.onHit`: award 3..=11 experience.
    fn award_experience(&self) {
        let Some(world) = self.level() else {
            return;
        };
        ExperienceOrbEntity::award(&world, self.position(), rand::random_range(3..=11));
    }
}

impl Entity for ExperienceBottleEntity {
    fn base(&self) -> &EntityBase {
        &self.base
    }

    fn entity_type(&self) -> EntityTypeRef {
        self.entity_type
    }

    fn tick(self: Arc<Self>) {
        self.throwable_projectile_tick();
    }

    fn get_default_gravity(&self) -> f64 {
        self.throwable_default_gravity()
    }

    fn sound_source(&self) -> SoundSource {
        SoundSource::Neutral
    }

    fn spawn_data(&self) -> i32 {
        self.get_owner().map_or(0, |owner| owner.id())
    }

    fn restore_owner_reference(&self, owner: &SharedEntity) {
        self.cache_owner_entity(owner);
    }

    fn projectile_owner_uuid(&self) -> Option<uuid::Uuid> {
        self.owner_uuid()
    }

    fn projectile_owner(&self) -> Option<SharedEntity> {
        self.get_owner()
    }

    fn synced_data(&self) -> Option<&dyn EntitySyncedData> {
        Some(&self.entity_data)
    }

    fn save_additional(&self, nbt: &mut NbtCompound) {
        self.save_projectile(nbt);
        self.save_throwable_item(nbt);
    }

    fn load_additional(&self, nbt: BorrowedNbtCompoundView<'_, '_>) {
        self.load_projectile(nbt);
        self.load_throwable_item(nbt);
    }
}

impl Projectile for ExperienceBottleEntity {
    fn projectile_base(&self) -> &ProjectileBase {
        &self.projectile_base
    }

    fn on_hit_entity(self: Arc<Self>, _entity: &SharedEntity, _location: DVec3) {
        self.award_experience();
        self.broadcast_entity_event(EntityStatus::Death);
        self.set_removed(RemovalReason::Discarded);
    }

    fn on_hit(self: Arc<Self>, hit: &ProjectileHit) {
        self.award_experience();
        Arc::clone(&self).projectile_on_hit(hit);
        self.broadcast_entity_event(EntityStatus::Death);
        self.set_removed(RemovalReason::Discarded);
    }
}

impl ThrowableProjectile for ExperienceBottleEntity {}

impl ThrowableItemProjectile for ExperienceBottleEntity {
    fn get_default_item(&self) -> ItemRef {
        &vanilla_items::EXPERIENCE_BOTTLE
    }

    fn set_item(&self, item: ItemStack) {
        self.entity_data
            .lock()
            .throwable_item_projectile
            .item_stack
            .set(item);
    }

    fn get_item(&self) -> ItemStack {
        self.entity_data
            .lock()
            .throwable_item_projectile
            .item_stack
            .get()
            .clone()
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Weak};

    use glam::DVec3;
    use steel_registry::{init_vanilla_registry, vanilla_entities};
    use steel_utils::{BlockPos, ChunkPos, Direction};

    use crate::behavior::init_behaviors;
    use crate::entity::{Entity, Projectile, ProjectileHit};
    use crate::test_support::{fresh_test_world, insert_ready_full_chunk};
    use crate::world::{ClipHitResult, World};

    use super::*;

    #[test]
    fn bottle_hit_awards_real_experience_orbs_and_discards_itself() {
        init_vanilla_registry();
        init_behaviors();
        let fixture = fresh_test_world("experience_bottle_hit");
        insert_ready_full_chunk(&fixture.world, ChunkPos::new(0, 0));
        let bottle = Arc::new(ExperienceBottleEntity::new(
            &vanilla_entities::EXPERIENCE_BOTTLE,
            1,
            DVec3::new(0.5, 65.0, 0.5),
            Arc::downgrade(&fixture.world),
        ));
        let hit = ProjectileHit::Block {
            location: DVec3::new(0.5, 64.0, 0.5),
            hit: ClipHitResult {
                location: DVec3::new(0.5, 64.0, 0.5),
                direction: Direction::Up,
                block_pos: BlockPos::new(0, 64, 0),
                miss: false,
                inside: false,
                world_border_hit: false,
            },
        };
        Arc::clone(&bottle).on_hit(&hit);
        assert!(bottle.is_removed(), "bottle must discard on impact");
        assert!(
            !fixture
                .world
                .get_entities_in_aabb_matching(
                    &steel_utils::WorldAabb::new(0.0, 64.0, 0.0, 1.0, 66.0, 1.0),
                    |entity| entity.entity_type() == &vanilla_entities::EXPERIENCE_ORB,
                )
                .is_empty(),
            "bottle must award real experience orbs"
        );
    }
}
