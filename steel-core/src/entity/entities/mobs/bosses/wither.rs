//! The Wither boss.
//!
//! Vanilla's wither is a flying monster with 300 health, a 220-tick spawn
//! invulnerability window, a ranged wither-skull attack, and a nether star
//! drop. The fight manager is the block pattern in `WitherSkullBlock`.

use std::sync::{Arc, Weak};

use glam::DVec3;
use rand::Rng;
use simdnbt::borrow::NbtCompound as BorrowedNbtCompoundView;
use simdnbt::owned::NbtCompound;
use steel_macros::entity_behavior;
use steel_protocol::packets::game::{CBossEvent, SoundSource};
use steel_registry::entity_type::{EntityDimensions, EntityTypeRef};
use steel_registry::sound_event::SoundEventRef;
use steel_registry::vanilla_entity_data::WitherBossEntityData;
use steel_registry::{vanilla_blocks, vanilla_entities, vanilla_items};
use steel_registry::blocks::block_state_ext::BlockStateExt as _;
use steel_registry::item_stack::ItemStack;
use steel_utils::translations;
use text_components::TextComponent;
use steel_utils::locks::SyncMutex;
use steel_utils::types::UpdateFlags;
use steel_utils::{BlockPos, BlockStateId, DowncastType, DowncastTypeKey};

use crate::entity::ai::goal::FloatGoal;
use crate::entity::damage::DamageSource;
use crate::entity::entities::WitherSkullEntity;
use crate::entity::{
    DefaultEntityPush, Entity, EntityBase, EntityBaseLoad, EntityMovementEmission, EntityPose,
    EntitySpawnReason, EntitySyncedData, LivingEntity, LivingEntityBase, Mob, MobBase, Monster,
    PathfinderMob, Projectile, SharedEntity, SpawnRule, next_entity_id, sync_dirty_mob_effects,
};
use crate::physics::MoveResult;
use crate::world::{LevelReader, World};

const DEFAULT_STEP_HEIGHT: f32 = 0.6;
const XP_REWARD: i32 = 50;
/// Vanilla's spawn invulnerability, in ticks.
const SPAWN_INVULNERABLE_TICKS: i32 = 220;
/// Vanilla's normal skull cadence.
const SKULL_INTERVAL: i32 = 40;
/// Vanilla fires twice as often once below half health.
const SKULL_INTERVAL_ENRAGED: i32 = 20;
const SKULL_BURST: usize = 1;
const SKULL_SPEED: f64 = 0.1;
const FLY_SPEED: f64 = 0.05;
const TARGET_RANGE_SQR: f64 = 64.0 * 64.0;
const ENRAGED_HEALTH_FRACTION: f32 = 0.5;

/// A wither boss entity.
#[entity_behavior(class = "WitherBoss", spawn_rule)]
pub struct WitherBossEntity {
    base: EntityBase,
    entity_type: EntityTypeRef,
    living_base: LivingEntityBase,
    mob_base: MobBase,
    entity_data: SyncMutex<WitherBossEntityData>,
    age: SyncMutex<i32>,
    attack_cooldown: SyncMutex<i32>,
    target: SyncMutex<Option<SharedEntity>>,
    boss_bar_sent: SyncMutex<bool>,
    boss_bar_progress: SyncMutex<f32>,
}

// SAFETY: The owner-scoped type key uniquely identifies WitherBossEntity.
unsafe impl DowncastType for WitherBossEntity {
    const TYPE_KEY: DowncastTypeKey = DowncastTypeKey::new("steel:entity/wither_boss");
}

impl WitherBossEntity {
    /// Withers are boss-spawned, never natural-spawned.
    pub(crate) const SPAWN_RULE: SpawnRule = Self::check_wither_spawn_rules;

    /// Only a summon pattern may create a wither.
    pub(crate) fn check_wither_spawn_rules(
        _entity_type: EntityTypeRef,
        _level: &dyn LevelReader,
        spawn_reason: EntitySpawnReason,
        _pos: BlockPos,
        _random: &mut dyn Rng,
    ) -> bool {
        matches!(
            spawn_reason,
            EntitySpawnReason::Command | EntitySpawnReason::Spawner | EntitySpawnReason::MobSummoned
        )
    }

    /// Creates a new wither entity.
    #[must_use]
    pub fn new(entity_type: EntityTypeRef, id: i32, position: DVec3, world: Weak<World>) -> Self {
        Self::new_with_base(
            EntityBase::new(id, position, entity_type.dimensions, world),
            entity_type,
        )
    }

    /// Loads a saved wither entity.
    #[must_use]
    pub fn from_saved(entity_type: EntityTypeRef, load: EntityBaseLoad) -> Self {
        Self::new_with_base(
            EntityBase::from_load(load, entity_type.dimensions),
            entity_type,
        )
    }

    fn new_with_base(base: EntityBase, entity_type: EntityTypeRef) -> Self {
        let living_base = LivingEntityBase::new(entity_type);
        let mob_base = MobBase::new();
        mob_base.set_xp_reward(XP_REWARD);
        let mut entity_data = WitherBossEntityData::new();
        living_base.initialize_synced_data(&mut entity_data);
        base.set_no_gravity(true);

        {
            let mut goals = mob_base.goal_selector().lock();
            goals.add_goal(0, FloatGoal::new(&mob_base));
        }

        Self {
            base,
            entity_type,
            living_base,
            mob_base,
            entity_data: SyncMutex::new(entity_data),
            age: SyncMutex::new(0),
            attack_cooldown: SyncMutex::new(SKULL_INTERVAL),
            target: SyncMutex::new(None),
            boss_bar_sent: SyncMutex::new(false),
            boss_bar_progress: SyncMutex::new(1.0),
        }
    }

    fn update_boss_bar(&self, world: &Arc<World>) {
        let progress = (self.get_health() / self.get_max_health().max(0.001)).clamp(0.0, 1.0);
        if !*self.boss_bar_sent.lock() {
            if world.players.len() == 0 {
                return;
            }
            *self.boss_bar_sent.lock() = true;
            *self.boss_bar_progress.lock() = progress;
            world.broadcast_to_all(CBossEvent::add(
                self.uuid(),
                TextComponent::translated(translations::ENTITY_MINECRAFT_WITHER.msg()),
                progress,
                5,
            ));
            return;
        }
        if *self.age.lock() % 20 != 0 {
            return;
        }
        let mut last = self.boss_bar_progress.lock();
        if (progress - *last).abs() > 0.01 {
            *last = progress;
            world.broadcast_to_all(CBossEvent::update_progress(self.uuid(), progress));
        }
    }

    fn update_dirty_mob_effect_entity_data(&self) {
        sync_dirty_mob_effects(&self.living_base, &self.entity_data);
    }

    fn enraged(&self) -> bool {
        self.get_health() <= self.get_max_health() * ENRAGED_HEALTH_FRACTION
    }

    fn nearest_player(&self, world: &Arc<World>) -> Option<SharedEntity> {
        let position = self.position();
        let mut best: Option<(f64, SharedEntity)> = None;
        world.players.iter_players(|_, player| {
            let candidate: SharedEntity = Arc::clone(player) as SharedEntity;
            let distance = candidate.position().distance_squared(position);
            if distance <= TARGET_RANGE_SQR
                && candidate.as_living_entity().is_some_and(LivingEntity::is_alive)
                && best.as_ref().is_none_or(|(best, _)| distance < *best)
            {
                best = Some((distance, candidate));
            }
            true
        });
        best.map(|(_, entity)| entity)
    }

    fn fly_and_attack(&self, world: &Arc<World>) {
        let mut cooldown = self.attack_cooldown.lock();
        *cooldown -= 1;

        let age = *self.age.lock();
        if age % 10 == 0 {
            *self.target.lock() = self.nearest_player(world);
        }
        let target = self.target.lock().clone();
        let Some(target) = target else {
            return;
        };

        let self_pos = self.position();
        let target_pos = target.position() + DVec3::new(0.0, 1.0, 0.0);
        let delta = target_pos - self_pos;
        let horizontal = DVec3::new(delta.x, 0.0, delta.z);
        let horizontal_speed = horizontal.length().max(0.001);
        let y_brake = (delta.y - 4.0).clamp(-FLY_SPEED, FLY_SPEED);
        let velocity = DVec3::new(
            horizontal.x / horizontal_speed * FLY_SPEED,
            y_brake,
            horizontal.z / horizontal_speed * FLY_SPEED,
        );
        self.set_velocity(velocity);
        self.mark_velocity_sync();
        self.look_at(crate::entity::EntityAnchor::Eyes, target_pos);

        if *cooldown > 0 {
            return;
        }
        *cooldown = if self.enraged() {
            SKULL_INTERVAL_ENRAGED
        } else {
            SKULL_INTERVAL
        };

        let head = self_pos + DVec3::new(0.0, self.bounding_box().height() * 0.5, 0.0);
        let mut direction = target_pos - head;
        if direction.length_squared() < 0.001 {
            direction = DVec3::new(0.0, 0.0, 1.0);
        }
        for index in 0..SKULL_BURST {
            let offset = if index == 0 {
                DVec3::ZERO
            } else {
                DVec3::new((index as f64 - 0.5) * 0.3, 0.0, 0.0)
            };
            let skull = WitherSkullEntity::new(
                &vanilla_entities::WITHER_SKULL,
                next_entity_id(),
                head + offset,
                Arc::downgrade(world),
            );
            skull.set_owner_uuid(Some(self.uuid()));
            skull.set_velocity(direction.normalize_or_zero() * SKULL_SPEED);
            skull.mark_velocity_sync();
            if let Err(error) = world.try_add_entity(Arc::new(skull) as SharedEntity) {
                log::debug!("failed to spawn wither skull: {error}");
            }
        }
    }
}

impl DefaultEntityPush for WitherBossEntity {}

impl Entity for WitherBossEntity {
    fn is_hard_collision_relevant(&self) -> bool {
        true
    }

    fn as_default_entity_push(&self) -> Option<&dyn DefaultEntityPush> {
        Some(self)
    }

    fn base(&self) -> &EntityBase {
        &self.base
    }

    fn entity_type(&self) -> EntityTypeRef {
        self.entity_type
    }

    fn movement_emission(&self) -> EntityMovementEmission {
        EntityMovementEmission::Events
    }

    fn base_tick(&self) {
        Mob::base_tick_mob(self);
        if let Some(world) = self.level() {
            *self.age.lock() += 1;
            self.fly_and_attack(&world);
            self.update_boss_bar(&world);
        }
    }

    fn dimensions_for_pose(&self, _pose: EntityPose) -> EntityDimensions {
        let scale = LivingEntity::get_scale(self);
        if self.entity_type.fixed {
            self.entity_type.dimensions
        } else {
            self.entity_type.dimensions.scale(scale)
        }
    }

    fn synced_data(&self) -> Option<&dyn EntitySyncedData> {
        Some(&self.entity_data)
    }

    fn update_data_before_sync(&self) {
        self.update_dirty_mob_effect_entity_data();
    }

    fn max_up_step(&self) -> f32 {
        self.attributes()
            .lock()
            .get_value(steel_registry::vanilla_attributes::STEP_HEIGHT)
            .unwrap_or(f64::from(DEFAULT_STEP_HEIGHT)) as f32
    }

    fn sound_source(&self) -> SoundSource {
        Monster::sound_source_monster(self)
    }

    fn fall_sounds(&self) -> (SoundEventRef, SoundEventRef) {
        Monster::fall_sounds_monster(self)
    }

    fn swim_sound(&self) -> SoundEventRef {
        Monster::swim_sound_monster(self)
    }

    fn swim_splash_sound(&self) -> SoundEventRef {
        Monster::swim_splash_sound_monster(self)
    }

    fn play_step_sound(&self, _pos: BlockPos, _block_state: BlockStateId) {}

    fn save_additional(&self, nbt: &mut NbtCompound) {
        self.save_mob(nbt);
    }

    fn load_additional(&self, nbt: BorrowedNbtCompoundView<'_, '_>) {
        self.load_mob(nbt);
    }
}

impl LivingEntity for WitherBossEntity {
    fn living_base(&self) -> &LivingEntityBase {
        &self.living_base
    }

    fn get_health(&self) -> f32 {
        *self.entity_data.lock().living_entity().health.get()
    }

    fn set_health(&self, health: f32) {
        let max_health = self.get_max_health();
        let clamped = health.clamp(0.0, max_health);
        self.entity_data
            .lock()
            .living_entity_mut()
            .health
            .set(clamped);
    }

    fn sound_volume(&self) -> f32 {
        1.0
    }

    fn is_invulnerable_to(&self, _world: &World, source: &DamageSource) -> bool {
        if *self.age.lock() < SPAWN_INVULNERABLE_TICKS {
            return true;
        }
        self.default_is_invulnerable_to(source)
    }

    fn drop_custom_death_loot(&self, _source: &DamageSource, _killed_by_player: bool) {
        let Some(world) = self.level() else {
            return;
        };
        if *self.boss_bar_sent.lock() {
            world.broadcast_to_all(CBossEvent::remove(self.uuid()));
            *self.boss_bar_sent.lock() = false;
        }
        let drop_pos = self.position() + DVec3::new(0.0, 0.5, 0.0);
        world.spawn_item(drop_pos, ItemStack::new(&vanilla_items::NETHER_STAR));

        // Vanilla plants a wither rose at the death position when the block
        // there can hold it.
        let rose_pos = self.block_position();
        if world.get_block_state(rose_pos).is_air() {
            world.set_block(
                rose_pos,
                vanilla_blocks::WITHER_ROSE.default_state(),
                UpdateFlags::UPDATE_ALL,
            );
        }
    }

    fn should_drop_experience(&self) -> bool {
        Monster::should_drop_experience_monster(self)
    }

    fn should_drop_loot(&self, world: &World) -> bool {
        Monster::should_drop_loot_monster(self, world)
    }

    fn server_ai_step(&self, entity: &SharedEntity) {
        Mob::mob_server_ai_step(self, entity);
    }

    fn ai_step(&self, entity: &SharedEntity) -> Option<MoveResult> {
        Monster::monster_ai_step(self, entity)
    }
}

impl Mob for WitherBossEntity {
    fn mob_base(&self) -> &MobBase {
        &self.mob_base
    }

    fn tick_goal_selectors(&self, entity: &SharedEntity) {
        PathfinderMob::tick_pathfinder_goal_selectors(self, entity);
    }

    fn tick_path_navigation(&self) {}

    fn mob_flags(&self) -> i8 {
        *self.entity_data.lock().mob().mob_flags.get()
    }

    fn set_mob_flags(&self, flags: i8) {
        self.entity_data.lock().mob_mut().mob_flags.set(flags);
    }
}

impl Monster for WitherBossEntity {}

impl PathfinderMob for WitherBossEntity {}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use glam::DVec3;
    use steel_registry::blocks::block_state_ext::BlockStateExt as _;
    use steel_registry::{
        init_vanilla_registry, vanilla_blocks, vanilla_damage_types, vanilla_entities,
    };
    use steel_utils::types::UpdateFlags;
    use steel_utils::{BlockPos, ChunkPos, Downcast as _};

    use super::WitherBossEntity;
    use crate::entity::damage::DamageSource;
    use crate::entity::{Entity, LivingEntity, SharedEntity, next_entity_id};
    use crate::test_support::{fresh_test_world, insert_ready_full_chunk};

    fn wither_world(key: &'static str) -> Arc<crate::world::World> {
        init_vanilla_registry();
        crate::behavior::init_behaviors();
        crate::block_entity::init_block_entities();
        // Leaked so the damage history backing a real killing blow stays alive.
        let fixture = Box::leak(Box::new(fresh_test_world(key)));
        insert_ready_full_chunk(&fixture.world, ChunkPos::new(0, 0));
        Arc::clone(&fixture.world)
    }

    #[test]
    fn wither_has_300_health_and_drops_a_nether_star() {
        let world = wither_world("wither_stats");
        let wither = Arc::new(WitherBossEntity::new(
            &vanilla_entities::WITHER,
            next_entity_id(),
            DVec3::new(0.5, 65.0, 0.5),
            Arc::downgrade(&world),
        ));
        assert_eq!(wither.get_max_health(), 300.0);
        assert_eq!(wither.get_health(), 300.0);

        let entity: SharedEntity = wither;
        world.try_add_entity(Arc::clone(&entity)).expect("wither add");
        let source = DamageSource::environment(&vanilla_damage_types::GENERIC);
        entity.as_living_entity().expect("living").drop_custom_death_loot(&source, true);

        let items = world.get_entities_in_aabb_matching(
            &steel_utils::WorldAabb::new(-2.0, 64.0, -2.0, 2.0, 66.0, 2.0),
            |entity| entity.downcast_ref::<crate::entity::entities::ItemEntity>().is_some(),
        );
        assert!(!items.is_empty(), "nether star item entity should be spawned");
    }

    #[test]
    fn a_real_killing_blow_drops_the_nether_star() {
        let world = wither_world("wither_real_death");
        let wither = Arc::new(WitherBossEntity::new(
            &vanilla_entities::WITHER,
            next_entity_id(),
            DVec3::new(0.5, 65.0, 0.5),
            Arc::downgrade(&world),
        ));
        let entity: SharedEntity = wither.clone();
        world.try_add_entity(Arc::clone(&entity)).expect("wither add");
        *wither.age.lock() = super::SPAWN_INVULNERABLE_TICKS;

        let source = DamageSource::environment(&vanilla_damage_types::EXPLOSION);
        assert!(
            wither.hurt_server(&world, &source, 400.0),
            "damage after the spawn window must land"
        );
        assert!(wither.is_dead_or_dying(), "the wither must die from the hit");

        let items = world.get_entities_in_aabb_matching(
            &steel_utils::WorldAabb::new(-2.0, 64.0, -2.0, 2.0, 66.0, 2.0),
            |entity| entity.downcast_ref::<crate::entity::entities::ItemEntity>().is_some(),
        );
        assert!(!items.is_empty(), "a real death must drop the nether star");
    }

    #[test]
    fn three_skulls_on_soul_sand_summon_the_wither() {
        let world = wither_world("wither_summon");
        for x in 0..3 {
            world.set_block(
                BlockPos::new(x, 64, 0),
                vanilla_blocks::SOUL_SAND.default_state(),
                UpdateFlags::UPDATE_ALL,
            );
        }
        for x in 0..3 {
            world.set_block(
                BlockPos::new(x, 65, 0),
                vanilla_blocks::WITHER_SKELETON_SKULL.default_state(),
                UpdateFlags::UPDATE_ALL,
            );
        }

        let withers = world.get_entities_in_aabb_matching(
            &steel_utils::WorldAabb::new(-2.0, 64.0, -2.0, 4.0, 70.0, 2.0),
            |entity| entity.downcast_ref::<WitherBossEntity>().is_some(),
        );
        assert_eq!(withers.len(), 1, "exactly one wither should be summoned");
        assert_eq!(
            world.get_block_state(BlockPos::new(1, 65, 0)).get_block(),
            &vanilla_blocks::AIR
        );
    }

    #[test]
    fn spawn_window_is_invulnerable_then_damageable() {
        let world = wither_world("wither_invuln");
        let wither = Arc::new(WitherBossEntity::new(
            &vanilla_entities::WITHER,
            next_entity_id(),
            DVec3::new(0.5, 65.0, 0.5),
            Arc::downgrade(&world),
        ));
        let source = DamageSource::environment(&vanilla_damage_types::GENERIC);
        assert!(wither.is_invulnerable_to(&world, &source));
        *wither.age.lock() = super::SPAWN_INVULNERABLE_TICKS;
        assert!(!wither.is_invulnerable_to(&world, &source));
    }
}
