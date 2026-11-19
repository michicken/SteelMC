use std::iter::empty;
use std::sync::{Arc, Weak};

use glam::DVec3;
use rand::Rng;
use simdnbt::borrow::NbtCompound as BorrowedNbtCompoundView;
use simdnbt::owned::NbtCompound;
use steel_macros::entity_behavior;
use steel_protocol::packets::game::SoundSource;
use steel_registry::entity_type::{EntityDimensions, EntityTypeRef};
use steel_registry::sound_event::SoundEventRef;
use steel_registry::vanilla_entity_data::GhastEntityData;
use steel_registry::item_stack::ItemStack;
use steel_registry::{sound_events, vanilla_attributes, vanilla_entities, vanilla_items};
use steel_utils::locks::SyncMutex;
use steel_utils::{BlockPos, BlockStateId, DowncastType, DowncastTypeKey};

use crate::entity::damage::DamageSource;
use crate::entity::{
    DefaultEntityPush, Entity, EntityBase, EntityBaseLoad, EntityMovementEmission, EntityPose,
    EntitySpawnReason, EntitySyncedData, LivingEntity, LivingEntityBase, Mob, MobBase, Monster,
    PathfinderMob, Projectile, SharedEntity, SpawnRule, next_entity_id, sync_dirty_mob_effects,
};
use crate::entity::entities::LargeFireballEntity;
use crate::physics::MoveResult;
use crate::world::{LevelReader, World};

const DEFAULT_STEP_HEIGHT: f32 = 0.6;
const XP_REWARD: i32 = 5;
const TARGET_RANGE_SQR: f64 = 64.0 * 64.0;
const FLY_SPEED: f64 = 0.035;
const FIREBALL_INTERVAL: i32 = 60;
const FIREBALL_SPEED: f64 = 0.1;

/// A ghast: a large flying nether mob that shoots explosive fireballs.
#[entity_behavior(class = "Ghast", spawn_rule)]
pub struct GhastEntity {
    base: EntityBase,
    entity_type: EntityTypeRef,
    living_base: LivingEntityBase,
    mob_base: MobBase,
    entity_data: SyncMutex<GhastEntityData>,
    age: SyncMutex<i32>,
    attack_cooldown: SyncMutex<i32>,
    target: SyncMutex<Option<SharedEntity>>,
}

// SAFETY: The owner-scoped type key uniquely identifies GhastEntity.
unsafe impl DowncastType for GhastEntity {
    const TYPE_KEY: DowncastTypeKey = DowncastTypeKey::new("steel:entity/ghast");
}

impl GhastEntity {
    /// Vanilla `Zombie.checkGhastSpawnRules` placeholder: monsters may spawn
    /// only when the spawn reason allows hostile natural spawning.
    pub(crate) const SPAWN_RULE: SpawnRule = Self::check_ghast_spawn_rules;

    /// Checks the ghast placement predicate.
    pub(crate) fn check_ghast_spawn_rules(
        _entity_type: EntityTypeRef,
        _level: &dyn LevelReader,
        spawn_reason: EntitySpawnReason,
        _pos: BlockPos,
        _random: &mut dyn Rng,
    ) -> bool {
        spawn_reason.is_spawner()
            || matches!(
                spawn_reason,
                EntitySpawnReason::Natural
                    | EntitySpawnReason::ChunkGeneration
                    | EntitySpawnReason::Patrol
                    | EntitySpawnReason::Reinforcement
            )
    }

    /// Creates a new zombie entity instance.
    #[must_use]
    pub fn new(entity_type: EntityTypeRef, id: i32, position: DVec3, world: Weak<World>) -> Self {
        Self::new_with_base(
            EntityBase::new(id, position, entity_type.dimensions, world),
            entity_type,
        )
    }

    /// Loads a saved zombie entity.
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
        let mut entity_data = GhastEntityData::new();
        living_base.initialize_synced_data(&mut entity_data);

        base.set_no_gravity(true);

        Self {
            base,
            entity_type,
            living_base,
            mob_base,
            entity_data: SyncMutex::new(entity_data),
            age: SyncMutex::new(0),
            attack_cooldown: SyncMutex::new(FIREBALL_INTERVAL),
            target: SyncMutex::new(None),
        }
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

        if *self.age.lock() % 20 == 0 {
            *self.target.lock() = self.nearest_player(world);
        }
        let target = self.target.lock().clone();
        let Some(target) = target else {
            self.set_velocity(DVec3::new(0.0, self.velocity().y * 0.5, 0.0));
            return;
        };

        let self_pos = self.position();
        let target_pos = target.position() + DVec3::new(0.0, 1.0, 0.0);
        let delta = target_pos - self_pos;
        let horizontal = DVec3::new(delta.x, 0.0, delta.z);
        let horizontal_speed = horizontal.length().max(0.001);
        let desired_distance = 16.0;
        let approach = if horizontal.length() < desired_distance {
            -FLY_SPEED
        } else {
            FLY_SPEED
        };
        let y_brake = (delta.y - 8.0).clamp(-FLY_SPEED, FLY_SPEED);
        self.set_velocity(DVec3::new(
            horizontal.x / horizontal_speed * approach,
            y_brake,
            horizontal.z / horizontal_speed * approach,
        ));
        self.mark_velocity_sync();
        self.look_at(crate::entity::EntityAnchor::Eyes, target_pos);

        if *cooldown > 0 {
            return;
        }
        *cooldown = FIREBALL_INTERVAL;

        let spawn = self_pos + DVec3::new(0.0, self.bounding_box().height() * 0.5, 0.0);
        let direction = (target_pos - spawn).normalize_or_zero();
        let fireball = LargeFireballEntity::new(
            &vanilla_entities::FIREBALL,
            next_entity_id(),
            spawn,
            Arc::downgrade(world),
        );
        fireball.set_owner_uuid(Some(self.uuid()));
        fireball.set_velocity(direction * FIREBALL_SPEED);
        fireball.mark_velocity_sync();
        if world.try_add_entity(Arc::new(fireball)).is_ok() {
            self.play_sound(&sound_events::ENTITY_GHAST_SHOOT, 1.0, 1.0);
        }
    }

    fn update_dirty_mob_effect_entity_data(&self) {
        sync_dirty_mob_effects(&self.living_base, &self.entity_data);
    }
}

impl DefaultEntityPush for GhastEntity {}

impl Entity for GhastEntity {
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
            .get_value(vanilla_attributes::STEP_HEIGHT)
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

    fn play_step_sound(&self, _pos: BlockPos, _block_state: BlockStateId) {
        self.play_sound(&sound_events::ENTITY_GHAST_AMBIENT, 0.15, 1.0);
    }

    fn save_additional(&self, nbt: &mut NbtCompound) {
        self.save_mob(nbt);
    }

    fn load_additional(&self, nbt: BorrowedNbtCompoundView<'_, '_>) {
        self.load_mob(nbt);
    }
}

impl LivingEntity for GhastEntity {
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
        0.4
    }

    fn hurt_sound(&self, _source: &DamageSource) -> Option<SoundEventRef> {
        Some(&sound_events::ENTITY_GHAST_HURT)
    }

    fn drop_custom_death_loot(&self, _source: &DamageSource, _killed_by_player: bool) {
        let Some(world) = self.level() else {
            return;
        };
        let drop_pos = self.position() + DVec3::new(0.0, 1.0, 0.0);
        world.spawn_item(drop_pos, ItemStack::new(&vanilla_items::GHAST_TEAR));
        world.spawn_item(drop_pos, ItemStack::new(&vanilla_items::GUNPOWDER));
    }

    fn death_sound(&self) -> Option<SoundEventRef> {
        Some(&sound_events::ENTITY_GHAST_DEATH)
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

impl Mob for GhastEntity {
    fn mob_base(&self) -> &MobBase {
        &self.mob_base
    }

    fn tick_goal_selectors(&self, entity: &SharedEntity) {
        PathfinderMob::tick_pathfinder_goal_selectors(self, entity);
    }

    fn tick_path_navigation(&self) {
        PathfinderMob::tick_pathfinder_path_navigation(self);
    }

    fn ambient_sound(&self) -> Option<SoundEventRef> {
        Some(&sound_events::ENTITY_GHAST_AMBIENT)
    }

    fn mob_flags(&self) -> i8 {
        *self.entity_data.lock().mob().mob_flags.get()
    }

    fn set_mob_flags(&self, flags: i8) {
        self.entity_data.lock().mob_mut().mob_flags.set(flags);
    }
}

impl Monster for GhastEntity {}

impl PathfinderMob for GhastEntity {}


#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use glam::DVec3;
    use steel_registry::{init_vanilla_registry, vanilla_entities};
    use steel_utils::ChunkPos;

    use super::GhastEntity;
    use crate::entity::{Entity, LivingEntity, next_entity_id};
    use crate::test_support::{TestPlayerBuilder, fresh_test_world, insert_ready_full_chunk};
    use crate::player::ResetReason;

    #[test]
    fn ghast_flies_and_shoots_a_large_fireball_at_a_nearby_player() {
        init_vanilla_registry();
        crate::behavior::init_behaviors();
        crate::block_entity::init_block_entities();
        let fixture = fresh_test_world("ghast_fireball");
        let world = &fixture.world;
        insert_ready_full_chunk(world, ChunkPos::new(0, 0));

        let player = TestPlayerBuilder::new(Arc::clone(world), "GhastTarget", 95).build();
        player.teleport_to(DVec3::new(8.5, 65.0, 8.5)).expect("player");
        assert!(world.add_player(player, ResetReason::InitialJoin));

        let ghast = GhastEntity::new(
            &vanilla_entities::GHAST,
            next_entity_id(),
            DVec3::new(8.5, 72.0, 8.5),
            Arc::downgrade(world),
        );
        assert_eq!(ghast.get_max_health(), 10.0);
        for _ in 0..70 {
            ghast.base_tick();
        }
        let fireballs = world.get_entities_in_aabb_matching(
            &steel_utils::WorldAabb::new(0.0, 60.0, 0.0, 16.0, 80.0, 16.0),
            |entity| entity.entity_type() == &vanilla_entities::FIREBALL,
        );
        assert!(!fireballs.is_empty(), "ghast must shoot a large fireball");
        assert!(ghast.base().no_gravity(), "ghast must not fall");
    }
}
