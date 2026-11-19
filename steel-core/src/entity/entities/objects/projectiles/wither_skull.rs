//! Wither skull projectile (`WitherSkull`).
//!
//! Deals 8 damage, inflicts 40 seconds of Wither, and explodes on impact.

use std::sync::Arc;
use std::sync::Weak;

use glam::DVec3;
use simdnbt::borrow::NbtCompound as BorrowedNbtCompoundView;
use simdnbt::owned::NbtCompound;
use steel_macros::entity_behavior;
use steel_registry::entity_type::EntityTypeRef;
use steel_registry::vanilla_damage_types;
use steel_registry::vanilla_entity_data::WitherSkullEntityData;
use steel_utils::locks::SyncMutex;
use steel_utils::{DowncastType, DowncastTypeKey, TICKS_PER_SECOND};

use crate::enchantment_helper::{self, EnchantmentPostAttackContext};
use crate::entity::damage::DamageSource;
use crate::entity::entity::EntityEventSource as _;
use crate::entity::{
    AbstractHurtingProjectile, Entity, EntityBase, EntityBaseLoad, EntitySyncedData,
    HurtingProjectileBase, MobEffectInstance, Projectile, ProjectileBase, ProjectileHit,
    RemovalReason, SharedEntity,
};
use crate::world::explosion::{ExplosionInteraction, ExplosionOptions};
use crate::world::{ClipHitResult, World};

const WITHER_SKULL_DAMAGE: f32 = 8.0;
const WITHER_EFFECT_TICKS: i32 = 40 * TICKS_PER_SECOND;
const WITHER_SKULL_EXPLOSION_POWER: f32 = 1.0;

/// A small fireball fired by a blaze.
#[entity_behavior(class = "WitherSkull")]
pub struct WitherSkullEntity {
    /// Common entity fields (id, uuid, position, etc.).
    base: EntityBase,
    /// Vanilla entity type registered for this implementation.
    entity_type: EntityTypeRef,
    /// Synced data carrying the rendered item stack.
    entity_data: SyncMutex<WitherSkullEntityData>,
    /// Shared `Projectile` state (owner / left-owner / has-been-shot).
    projectile_base: ProjectileBase,
    /// Shared `AbstractHurtingProjectile` state (acceleration power).
    hurting_projectile_base: HurtingProjectileBase,
}

// SAFETY: This key is owned by Steel and uniquely identifies `WitherSkullEntity`.
unsafe impl DowncastType for WitherSkullEntity {
    const TYPE_KEY: DowncastTypeKey = DowncastTypeKey::new("steel:entity/wither_skull");
}

impl WitherSkullEntity {
    /// Creates a new small fireball with no owner.
    #[must_use]
    pub fn new(entity_type: EntityTypeRef, id: i32, position: DVec3, world: Weak<World>) -> Self {
        Self::with_base(
            entity_type,
            EntityBase::new(id, position, entity_type.dimensions, world),
        )
    }

    /// Creates a small fireball from saved base data.
    #[must_use]
    pub fn from_saved(entity_type: EntityTypeRef, load: EntityBaseLoad) -> Self {
        Self::with_base(
            entity_type,
            EntityBase::from_load(load, entity_type.dimensions),
        )
    }

    fn with_base(entity_type: EntityTypeRef, base: EntityBase) -> Self {
        Self {
            base,
            entity_type,
            entity_data: SyncMutex::new(WitherSkullEntityData::new()),
            projectile_base: ProjectileBase::new(),
            hurting_projectile_base: HurtingProjectileBase::default(),
        }
    }
}

impl Entity for WitherSkullEntity {
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

    fn synced_data(&self) -> Option<&dyn EntitySyncedData> {
        Some(&self.entity_data)
    }

    fn play_entity_on_fire_extinguished_sound(&self) {}

    fn save_additional(&self, nbt: &mut NbtCompound) {
        self.save_hurting_projectile(nbt);
    }

    fn load_additional(&self, nbt: BorrowedNbtCompoundView<'_, '_>) {
        self.load_hurting_projectile(nbt);
    }
}

impl Projectile for WitherSkullEntity {
    fn projectile_base(&self) -> &ProjectileBase {
        &self.projectile_base
    }

    fn can_hit_entity(&self, entity: &dyn Entity) -> bool {
        self.hurting_can_hit_entity(entity)
    }

    fn on_deflection(&self, by_attack: bool) {
        self.hurting_on_deflection(by_attack);
    }

    fn on_hit(self: Arc<Self>, hit: &ProjectileHit) {
        Arc::clone(&self).projectile_on_hit(hit);
        self.set_removed(RemovalReason::Discarded);
    }

    fn on_hit_entity(self: Arc<Self>, entity: &SharedEntity, _location: DVec3) {
        let Some(world) = entity.level() else {
            return;
        };

        let owner = self.get_owner();
        let mut source = DamageSource::environment(&vanilla_damage_types::MAGIC)
            .with_direct_entity(Arc::<Self>::clone(&self).into_shared_entity());
        if let Some(owner) = owner {
            source = source.with_causing_entity(owner);
        }

        if entity.hurt(&world, &source, WITHER_SKULL_DAMAGE) {
            let context = EnchantmentPostAttackContext::new(entity, &source);
            enchantment_helper::do_post_attack_effects(&world, entity.as_ref(), &context);
            if let Some(living) = entity.as_living_entity() {
                living.add_mob_effect(MobEffectInstance::with_duration(
                    &steel_registry::vanilla_mob_effects::WITHER,
                    WITHER_EFFECT_TICKS,
                    0,
                ));
            }
        }

        let mut options = ExplosionOptions::new(
            self.position(),
            WITHER_SKULL_EXPLOSION_POWER,
            ExplosionInteraction::Mob,
        );
        if let Some(owner) = self.get_owner() {
            options.source = Some(owner);
        }
        world.explode(options);
    }

    fn on_hit_block(self: Arc<Self>, hit: &ClipHitResult) {
        self.projectile_on_hit_block(hit);
        let Some(world) = self.level() else {
            return;
        };

        let _ = hit;
        let mut options = ExplosionOptions::new(
            self.position(),
            WITHER_SKULL_EXPLOSION_POWER,
            ExplosionInteraction::Mob,
        );
        if let Some(owner) = self.get_owner() {
            options.source = Some(owner);
        }
        world.explode(options);
    }

    fn hurt(&self, _world: &World, _source: &DamageSource, _amount: f32) -> bool {
        false
    }
}

impl AbstractHurtingProjectile for WitherSkullEntity {
    fn hurting_projectile_base(&self) -> &HurtingProjectileBase {
        &self.hurting_projectile_base
    }
}

