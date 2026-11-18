//! Small fireball projectile entity (`SmallFireball`), used by fire charges.

use std::sync::Arc;
use std::sync::Weak;

use glam::DVec3;
use simdnbt::borrow::NbtCompound as BorrowedNbtCompoundView;
use simdnbt::owned::NbtCompound;
use steel_macros::entity_behavior;
use steel_protocol::packets::game::SoundSource;
use steel_registry::blocks::block_state_ext::BlockStateExt as _;
use steel_registry::entity_type::EntityTypeRef;
use steel_registry::{vanilla_blocks, vanilla_damage_types};
use steel_utils::types::UpdateFlags;
use steel_utils::{DowncastType, DowncastTypeKey};

use crate::behavior::blocks::FireBlock;
use crate::entity::damage::DamageSource;
use crate::entity::{
    AbstractHurtingProjectile, Entity, EntityBase, EntityBaseLoad, HurtingProjectileBase, Projectile,
    ProjectileBase, ProjectileHit, RemovalReason, SharedEntity, next_entity_id,
};
use crate::world::World;
use crate::world::ClipHitResult;

/// A small fireball (fire charge projectile).
#[entity_behavior(class = "SmallFireball")]
pub struct SmallFireballEntity {
    base: EntityBase,
    entity_type: EntityTypeRef,
    projectile_base: ProjectileBase,
    hurting_projectile_base: HurtingProjectileBase,
}

// SAFETY: This key is owned by Steel and uniquely identifies `SmallFireballEntity`.
unsafe impl DowncastType for SmallFireballEntity {
    const TYPE_KEY: DowncastTypeKey = DowncastTypeKey::new("steel:entity/small_fireball");
}

impl SmallFireballEntity {
    #[must_use]
    pub fn new(entity_type: EntityTypeRef, id: i32, position: DVec3, world: Weak<World>) -> Self {
        Self {
            base: EntityBase::new(id, position, entity_type.dimensions, world),
            entity_type,
            projectile_base: ProjectileBase::new(),
            hurting_projectile_base: HurtingProjectileBase::default(),
        }
    }

    #[must_use]
    pub fn from_saved(entity_type: EntityTypeRef, load: EntityBaseLoad) -> Self {
        Self {
            base: EntityBase::from_load(load, entity_type.dimensions),
            entity_type,
            projectile_base: ProjectileBase::new(),
            hurting_projectile_base: HurtingProjectileBase::default(),
        }
    }

    fn ignite_hit_block(&self, hit: &ClipHitResult) {
        let Some(world) = self.level() else {
            return;
        };
        let pos = hit.block_pos.relative(hit.direction);
        if FireBlock::can_be_placed_at(&world, pos, hit.direction) {
            world.set_block(
                pos,
                FireBlock::get_state(world.as_ref(), pos),
                UpdateFlags::UPDATE_ALL,
            );
        }
    }
}

impl Entity for SmallFireballEntity {
    fn base(&self) -> &EntityBase {
        &self.base
    }

    fn entity_type(&self) -> EntityTypeRef {
        self.entity_type
    }

    fn tick(self: Arc<Self>) {
        self.hurting_projectile_tick();
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
        self.save_hurting_projectile(nbt);
    }

    fn load_additional(&self, nbt: BorrowedNbtCompoundView<'_, '_>) {
        self.load_hurting_projectile(nbt);
    }
}

impl Projectile for SmallFireballEntity {
    fn projectile_base(&self) -> &ProjectileBase {
        &self.projectile_base
    }

    fn on_hit(self: Arc<Self>, hit: &ProjectileHit) {
        Arc::clone(&self).projectile_on_hit(hit);
    }

    fn on_hit_entity(self: Arc<Self>, entity: &SharedEntity, _location: DVec3) {
        let Some(world) = entity.level() else {
            return;
        };
        let mut damage =
            DamageSource::environment(&vanilla_damage_types::FIREBALL)
                .with_direct_entity(Arc::clone(&self) as SharedEntity);
        if let Some(owner) = self.get_owner() {
            damage = damage.with_causing_entity(owner);
        }
        entity.hurt(&world, &damage, 5.0);
        entity.ignite_for_ticks(100);
        self.set_removed(RemovalReason::Discarded);
    }

    fn on_hit_block(self: Arc<Self>, hit: &ClipHitResult) {
        self.ignite_hit_block(hit);
        self.set_removed(RemovalReason::Discarded);
    }
}

impl AbstractHurtingProjectile for SmallFireballEntity {
    fn hurting_projectile_base(&self) -> &HurtingProjectileBase {
        &self.hurting_projectile_base
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use glam::DVec3;
    use steel_registry::{init_vanilla_registry, vanilla_blocks, vanilla_entities};
    use steel_utils::types::UpdateFlags;
    use steel_utils::{BlockPos, ChunkPos, Direction};

    use crate::behavior::init_behaviors;
    use crate::entity::{Entity, Projectile};
    use crate::test_support::{fresh_test_world, insert_ready_full_chunk};
    use crate::world::ClipHitResult;

    use super::*;

    #[test]
    fn block_hit_places_real_fire_on_the_adjacent_face() {
        init_vanilla_registry();
        init_behaviors();
        let fixture = fresh_test_world("small_fireball_block_hit");
        insert_ready_full_chunk(&fixture.world, ChunkPos::new(0, 0));
        assert!(fixture.world.set_block(
            BlockPos::new(8, 63, 8),
            vanilla_blocks::STONE.default_state(),
            UpdateFlags::UPDATE_NONE,
        ));
        let fireball = Arc::new(SmallFireballEntity::new(
            &vanilla_entities::SMALL_FIREBALL,
            1,
            DVec3::new(8.5, 65.0, 8.5),
            Arc::downgrade(&fixture.world),
        ));
        let hit = ClipHitResult {
            location: DVec3::new(8.5, 64.0, 8.5),
            direction: Direction::Up,
            block_pos: BlockPos::new(8, 63, 8),
            miss: false,
            inside: false,
            world_border_hit: false,
        };
        Arc::clone(&fireball).on_hit_block(&hit);
        assert_eq!(
            fixture.world.get_block_state(BlockPos::new(8, 64, 8)).get_block(),
            &vanilla_blocks::FIRE,
            "small fireball must place fire on impact"
        );
        assert!(fireball.is_removed(), "small fireball must discard on impact");
    }
}
