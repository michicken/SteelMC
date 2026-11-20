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
use steel_registry::item_stack::ItemStack;
use steel_registry::vanilla_entity_data::SkeletonEntityData;
use steel_registry::{sound_events, vanilla_attributes, vanilla_entities, vanilla_items};
use steel_utils::locks::SyncMutex;
use steel_utils::{BlockPos, BlockStateId, DowncastType, DowncastTypeKey};

use crate::entity::ai::goal::{
    FloatGoal, Goal, GoalControls, HurtByTargetGoal, LookAtPlayerGoal, MeleeAttackGoal,
    NearestAttackableTargetGoal, RandomLookAroundGoal, WaterAvoidingRandomStrollGoal,
};
use crate::entity::entities::objects::projectiles::ArrowEntity;
use crate::entity::damage::DamageSource;
use crate::inventory::equipment::EquipmentSlot;
use crate::entity::{
    DefaultEntityPush, Entity, EntityBase, EntityBaseLoad, EntityMovementEmission, EntityPose,
    EntitySpawnReason, EntitySyncedData, LivingEntity, LivingEntityBase, Mob, MobBase, Monster,
    PathfinderMob, Projectile, SharedEntity, SpawnRule, next_entity_id, sync_dirty_mob_effects,
};
use crate::physics::MoveResult;
use crate::world::{LevelReader, World};

const DEFAULT_STEP_HEIGHT: f32 = 0.6;
const XP_REWARD: i32 = 5;
const BOW_RANGE_SQR: f64 = 15.0 * 15.0;
/// Vanilla `RangedBowAttackGoal` draws for 20 ticks, then waits its
/// `attackIntervalMin` (20 ticks) before starting the next draw.
const BOW_DRAW_TICKS: i32 = 20;
const BOW_COOLDOWN_TICKS: i32 = 20;
const ARROW_SPEED: f64 = 2.0;
const ARROW_ARC_LIFT: f64 = 0.12;

/// Vanilla-shaped skeleton bow attack: keeps its distance and fires an arrow
/// every 20 ticks at the current target.
struct BowAttackGoal {
    cooldown: i32,
    draw_ticks: i32,
}

impl BowAttackGoal {
    const fn new() -> Self {
        // Do not fire on the first AI tick. Vanilla needs a full draw first.
        Self {
            cooldown: BOW_DRAW_TICKS,
            draw_ticks: 0,
        }
    }

    fn fire(&mut self, mob: &dyn PathfinderMob, entity: &SharedEntity) -> Option<SharedEntity> {
        let target = mob.target()?;
        let world = mob.level()?;
        let origin = DVec3::new(mob.position().x, mob.get_eye_y(), mob.position().z);
        let target_eye = DVec3::new(
            target.position().x,
            target.get_eye_y(),
            target.position().z,
        );
        let mut direction = (target_eye - origin).normalize_or_zero();
        if !direction.is_finite() || direction.length_squared() < 1.0e-9 {
            return None;
        }
        direction.y += ARROW_ARC_LIFT;

        let arrow_typed = Arc::new(ArrowEntity::new(
            &vanilla_entities::ARROW,
            next_entity_id(),
            origin + direction * 0.5,
            Arc::downgrade(&world),
        ));
        arrow_typed.set_owner_entity(Some(entity));
        arrow_typed.set_velocity(direction * ARROW_SPEED);
        arrow_typed.mark_velocity_sync();
        let arrow: SharedEntity = arrow_typed;
        world.try_add_entity(Arc::clone(&arrow)).ok()?;
        Some(arrow)
    }
}

impl Goal for BowAttackGoal {
    fn controls(&self) -> GoalControls {
        GoalControls::MOVE | GoalControls::LOOK
    }

    fn can_use(&mut self, mob: &dyn PathfinderMob) -> bool {
        let Some(target) = mob.target() else {
            return false;
        };
        mob.position().distance_squared(target.position()) <= BOW_RANGE_SQR
            && mob.has_line_of_sight_cached(target.as_ref())
    }

    fn tick(&mut self, mob: &dyn PathfinderMob, entity: &SharedEntity) {
        if self.cooldown > 0 {
            self.cooldown -= 1;
            self.draw_ticks = 0;
            return;
        }
        if mob.target().is_none() {
            self.draw_ticks = 0;
            return;
        }
        // Draw first, release on the 20th tick, then enter the 20-tick
        // cooldown. This produces vanilla's 40-tick cycle instead of the
        // previous one-arrow-every-20-ticks behavior.
        self.draw_ticks = self.draw_ticks.saturating_add(1);
        if self.draw_ticks < BOW_DRAW_TICKS {
            return;
        }
        if self.fire(mob, entity).is_some() {
            self.draw_ticks = 0;
            self.cooldown = BOW_COOLDOWN_TICKS;
        }
    }

    fn requires_update_every_tick(&self) -> bool {
        true
    }
}

/// A hostile skeleton entity.
#[entity_behavior(class = "Skeleton", spawn_rule)]
pub struct SkeletonEntity {
    base: EntityBase,
    entity_type: EntityTypeRef,
    living_base: LivingEntityBase,
    mob_base: MobBase,
    entity_data: SyncMutex<SkeletonEntityData>,
}

// SAFETY: The owner-scoped type key uniquely identifies SkeletonEntity.
unsafe impl DowncastType for SkeletonEntity {
    const TYPE_KEY: DowncastTypeKey = DowncastTypeKey::new("steel:entity/skeleton");
}

impl SkeletonEntity {
    /// Vanilla `AbstractSkeleton.checkSkeletonSpawnRules` placeholder: monsters may spawn
    /// only when the spawn reason allows hostile natural spawning.
    pub(crate) const SPAWN_RULE: SpawnRule = Self::check_skeleton_spawn_rules;

    /// Checks the Skeleton placement predicate.
    pub(crate) fn check_skeleton_spawn_rules(
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

    /// Creates a new skeleton entity instance.
    #[must_use]
    pub fn new(entity_type: EntityTypeRef, id: i32, position: DVec3, world: Weak<World>) -> Self {
        Self::new_with_base(
            EntityBase::new(id, position, entity_type.dimensions, world),
            entity_type,
        )
    }

    /// Loads a saved skeleton entity.
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
        let mut entity_data = SkeletonEntityData::new();
        living_base.initialize_synced_data(&mut entity_data);
        // Vanilla skeletons spawn holding a bow. Without this the ranged
        // goal still fired arrows but the model showed an empty hand.
        living_base
            .equipment()
            .lock()
            .set(EquipmentSlot::MainHand, ItemStack::new(&vanilla_items::BOW));

        {
            let mut goal_selector = mob_base.goal_selector().lock();
            goal_selector.add_goal(0, FloatGoal::new(&mob_base));
            goal_selector.add_goal(2, BowAttackGoal::new());
            goal_selector.add_goal(3, MeleeAttackGoal::new(1.0, false));
            goal_selector.add_goal(7, WaterAvoidingRandomStrollGoal::new(1.0));
            goal_selector.add_goal(8, LookAtPlayerGoal::new(8.0));
            goal_selector.add_goal(8, RandomLookAroundGoal::new());

            let mut target_selector = mob_base.target_selector().lock();
            target_selector.add_goal(1, HurtByTargetGoal::new().set_alert_others(empty()));
            target_selector.add_goal(
                2,
                NearestAttackableTargetGoal::new_for_players(true, |_, _| true),
            );
        }

        Self {
            base,
            entity_type,
            living_base,
            mob_base,
            entity_data: SyncMutex::new(entity_data),
        }
    }

    fn update_dirty_mob_effect_entity_data(&self) {
        sync_dirty_mob_effects(&self.living_base, &self.entity_data);
    }
}

impl DefaultEntityPush for SkeletonEntity {}

impl Entity for SkeletonEntity {
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
        self.play_sound(&sound_events::ENTITY_SKELETON_STEP, 0.15, 1.0);
    }

    fn save_additional(&self, nbt: &mut NbtCompound) {
        self.save_mob(nbt);
    }

    fn load_additional(&self, nbt: BorrowedNbtCompoundView<'_, '_>) {
        self.load_mob(nbt);
    }
}

impl LivingEntity for SkeletonEntity {
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
        Some(&sound_events::ENTITY_SKELETON_HURT)
    }

    fn death_sound(&self) -> Option<SoundEventRef> {
        Some(&sound_events::ENTITY_SKELETON_DEATH)
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

impl Mob for SkeletonEntity {
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
        Some(&sound_events::ENTITY_SKELETON_AMBIENT)
    }

    fn mob_flags(&self) -> i8 {
        *self.entity_data.lock().mob().mob_flags.get()
    }

    fn set_mob_flags(&self, flags: i8) {
        self.entity_data.lock().mob_mut().mob_flags.set(flags);
    }
}

impl Monster for SkeletonEntity {}

impl PathfinderMob for SkeletonEntity {}


#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use glam::DVec3;
    use steel_registry::{init_vanilla_registry, vanilla_entities};
    use steel_registry::blocks::block_state_ext::BlockStateExt as _;
    use steel_registry::vanilla_blocks;
    use steel_utils::{BlockPos, ChunkPos, Downcast as _};

    use super::SkeletonEntity;
    use crate::entity::{Entity, LivingEntity, SharedEntity, next_entity_id};
    use crate::test_support::{
        TestPlayerBuilder, fresh_test_world, insert_ready_full_chunk, tick_test_world,
    };

    #[test]
    fn skeleton_entity_registers_spawns_and_keeps_vanilla_health() {
        init_vanilla_registry();
        crate::behavior::init_behaviors();
        crate::block_entity::init_block_entities();
        let fixture = fresh_test_world("skeleton_spawn");
        let world = &fixture.world;
        insert_ready_full_chunk(world, ChunkPos::new(0, 0));

        let skeleton: SharedEntity = Arc::new(SkeletonEntity::new(
            &vanilla_entities::SKELETON,
            next_entity_id(),
            DVec3::new(8.5, 65.0, 8.5),
            Arc::downgrade(world),
        ));
        world
            .try_add_entity(Arc::clone(&skeleton))
            .expect("skeleton should attach to the loaded chunk");
        let skeleton_entity = skeleton
            .downcast_ref::<SkeletonEntity>()
            .expect("skeleton is concrete");

        assert_eq!(skeleton_entity.entity_type(), &vanilla_entities::SKELETON);
        assert_eq!(skeleton_entity.entity_type().key.path, "skeleton");
        assert_eq!(skeleton_entity.get_health(), 20.0);
        for _ in 0..10 {
            tick_test_world(world, 1, true);
            skeleton_entity.ai_step(&skeleton);
            skeleton_entity.base_tick();
        }
        assert!(Entity::is_alive(skeleton_entity), "skeleton must survive its first ticks");
    }

    #[test]
    fn skeleton_bow_ai_shoots_real_arrows_at_a_player() {
        init_vanilla_registry();
        crate::behavior::init_behaviors();
        crate::block_entity::init_block_entities();
        let fixture = fresh_test_world("skeleton_bow");
        let world = &fixture.world;
        insert_ready_full_chunk(world, ChunkPos::new(0, 0));
        for x in 0..20 {
            for z in 0..16 {
                world.set_block(
                    BlockPos::new(x, 64, z),
                    vanilla_blocks::STONE.default_state(),
                    steel_utils::types::UpdateFlags::UPDATE_CLIENTS,
                );
            }
        }

        let player = TestPlayerBuilder::new(Arc::clone(world), "SkelBowTarget", 9).build();
        player
            .teleport_to(DVec3::new(13.5, 65.0, 8.5))
            .expect("player should teleport onto the platform");
        assert!(world.add_player(player.clone(), crate::player::ResetReason::InitialJoin));

        let skeleton: SharedEntity = Arc::new(SkeletonEntity::new(
            &vanilla_entities::SKELETON,
            next_entity_id(),
            DVec3::new(8.5, 65.0, 8.5),
            Arc::downgrade(world),
        ));
        world
            .try_add_entity(Arc::clone(&skeleton))
            .expect("skeleton should attach to the loaded chunk");
        let skeleton_entity = skeleton
            .downcast_ref::<SkeletonEntity>()
            .expect("skeleton is concrete");

        for _ in 0..60 {
            tick_test_world(world, 1, true);
            skeleton_entity.ai_step(&skeleton);
            skeleton_entity.base_tick();
        }

        let arrows = world.get_entities_in_aabb_matching(
            &steel_utils::WorldAabb::new(5.0, 64.0, 5.0, 16.0, 70.0, 12.0),
            Entity::is_abstract_arrow,
        );
        assert!(
            !arrows.is_empty(),
            "skeleton bow AI must spawn at least one arrow entity"
        );
    }
}
