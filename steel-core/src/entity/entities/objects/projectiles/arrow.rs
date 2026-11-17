//! Arrow projectile entity (`Arrow`).
//!
//! Implements the shared `Projectile → ThrowableProjectile` stack with
//! arrow-specific drag/gravity and vanilla arrow damage on entity impact.
//! Pickup, critical hits and tipped-arrow effects are still future work.

use std::sync::Arc;
use std::sync::Weak;

use glam::DVec3;
use simdnbt::borrow::NbtCompound as BorrowedNbtCompoundView;
use simdnbt::owned::NbtCompound;
use steel_macros::entity_behavior;
use steel_protocol::packets::game::SoundSource;
use steel_registry::entity_type::EntityTypeRef;
use steel_registry::{vanilla_damage_types, vanilla_entities};
use steel_utils::{DowncastType, DowncastTypeKey};

use crate::entity::damage::DamageSource;
use crate::entity::{
    Entity, EntityBase, EntityBaseLoad, Projectile, ProjectileBase, ProjectileHit, RemovalReason,
    SharedEntity, ThrowableProjectile,
};
use crate::world::World;

const ARROW_GRAVITY: f64 = 0.05;
const ARROW_AIR_DRAG: f32 = 0.99;
const ARROW_BASE_DAMAGE: f32 = 2.0;

/// A shot arrow.
#[entity_behavior(class = "Arrow")]
pub struct ArrowEntity {
    base: EntityBase,
    entity_type: EntityTypeRef,
    projectile_base: ProjectileBase,
}

// SAFETY: This key uniquely identifies `ArrowEntity`.
unsafe impl DowncastType for ArrowEntity {
    const TYPE_KEY: DowncastTypeKey = DowncastTypeKey::new("steel:entity/arrow");
}

impl ArrowEntity {
    /// Creates a new arrow with no owner.
    #[must_use]
    pub fn new(entity_type: EntityTypeRef, id: i32, position: DVec3, world: Weak<World>) -> Self {
        Self {
            base: EntityBase::new(id, position, entity_type.dimensions, world),
            entity_type,
            projectile_base: ProjectileBase::new(),
        }
    }

    /// Creates an arrow from saved base data.
    #[must_use]
    pub fn from_saved(entity_type: EntityTypeRef, load: EntityBaseLoad) -> Self {
        Self {
            base: EntityBase::from_load(load, entity_type.dimensions),
            entity_type,
            projectile_base: ProjectileBase::new(),
        }
    }
}

impl Entity for ArrowEntity {
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

    fn save_additional(&self, nbt: &mut NbtCompound) {
        self.save_projectile(nbt);
    }

    fn load_additional(&self, nbt: BorrowedNbtCompoundView<'_, '_>) {
        self.load_projectile(nbt);
    }
}

impl Projectile for ArrowEntity {
    fn projectile_base(&self) -> &ProjectileBase {
        &self.projectile_base
    }

    fn on_hit_entity(self: Arc<Self>, entity: &SharedEntity, _location: DVec3) {
        let mut damage = DamageSource::environment(&vanilla_damage_types::ARROW)
            .with_direct_entity(Arc::<Self>::clone(&self));
        if let Some(owner) = self.get_owner() {
            damage = damage.with_causing_entity(owner);
        }
        if let Some(world) = entity.level() {
            entity.hurt(&world, &damage, ARROW_BASE_DAMAGE);
        }
        self.set_removed(RemovalReason::Discarded);
    }

    fn on_hit(self: Arc<Self>, hit: &ProjectileHit) {
        Arc::clone(&self).projectile_on_hit(hit);
        self.set_removed(RemovalReason::Discarded);
    }
}

impl ThrowableProjectile for ArrowEntity {
    fn get_air_drag(&self) -> f32 {
        ARROW_AIR_DRAG
    }

    fn throwable_default_gravity(&self) -> f64 {
        ARROW_GRAVITY
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use glam::DVec3;
    use steel_registry::blocks::block_state_ext::BlockStateExt as _;
    use steel_registry::{init_vanilla_registry, vanilla_blocks, vanilla_entities};
    use steel_utils::{BlockPos, ChunkPos, Downcast as _};

    use super::ArrowEntity;
    use crate::entity::entities::mobs::hostile::ZombieEntity;
    use crate::entity::{Entity, LivingEntity, Projectile, SharedEntity, next_entity_id};
    use crate::test_support::{fresh_test_world, insert_ready_full_chunk};

    #[test]
    fn arrow_shoots_and_damages_a_player() {
        init_vanilla_registry();
        crate::behavior::init_behaviors();
        crate::block_entity::init_block_entities();
        let fixture = fresh_test_world("arrow_hits_player");
        let world = &fixture.world;
        insert_ready_full_chunk(world, ChunkPos::new(0, 0));
        for x in 0..20 {
            for z in 0..20 {
                world.set_block(
                    BlockPos::new(x, 64, z),
                    vanilla_blocks::STONE.default_state(),
                    steel_utils::types::UpdateFlags::UPDATE_CLIENTS,
                );
            }
        }

        let target: SharedEntity = Arc::new(ZombieEntity::new(
            &vanilla_entities::ZOMBIE,
            next_entity_id(),
            DVec3::new(8.5, 65.0, 12.5),
            Arc::downgrade(world),
        ));
        world
            .try_add_entity(Arc::clone(&target))
            .expect("zombie target should attach to the loaded chunk");
        let target_entity = target
            .downcast_ref::<ZombieEntity>()
            .expect("zombie target is concrete");

        let arrow: SharedEntity = Arc::new(ArrowEntity::new(
            &vanilla_entities::ARROW,
            next_entity_id(),
            DVec3::new(8.5, 66.5, 8.5),
            Arc::downgrade(world),
        ));
        world
            .try_add_entity(Arc::clone(&arrow))
            .expect("arrow should attach to the loaded chunk");
        let arrow_entity = arrow.downcast_ref::<ArrowEntity>().expect("arrow is concrete");
        arrow_entity.set_velocity(DVec3::new(0.0, 0.0, 1.5));
        arrow_entity.mark_velocity_sync();

        for _ in 0..12 {
            if !Entity::is_alive(arrow_entity) {
                break;
            }
            Entity::tick(Arc::clone(&arrow));
        }
        assert!(
            target_entity.get_health() < 20.0,
            "arrow must damage the mob in its path, health={}, arrow_pos={:?}, target_pos={:?}, arrow_alive={}",
            target_entity.get_health(),
            arrow.position(),
            target.position(),
            Entity::is_alive(arrow_entity)
        );
    }
}
