use std::iter::empty;
use std::sync::Weak;

use glam::DVec3;
use rand::Rng;
use simdnbt::borrow::NbtCompound as BorrowedNbtCompoundView;
use simdnbt::owned::NbtCompound;
use steel_macros::entity_behavior;
use steel_protocol::packets::game::SoundSource;
use steel_registry::entity_type::{EntityDimensions, EntityTypeRef};
use steel_registry::sound_event::SoundEventRef;
use steel_registry::vanilla_entity_data::SpiderEntityData;
use steel_registry::{sound_events, vanilla_attributes};
use steel_utils::locks::SyncMutex;
use steel_utils::{BlockPos, BlockStateId, DowncastType, DowncastTypeKey};

use crate::entity::ai::goal::{
    FloatGoal, HurtByTargetGoal, LeapAtTargetGoal, LookAtPlayerGoal, MeleeAttackGoal,
    NearestAttackableTargetGoal, RandomLookAroundGoal, WaterAvoidingRandomStrollGoal,
};
use crate::entity::damage::DamageSource;
use crate::entity::{
    DefaultEntityPush, Entity, EntityBase, EntityBaseLoad, EntityMovementEmission, EntityPose,
    EntitySpawnReason, EntitySyncedData, LivingEntity, LivingEntityBase, Mob, MobBase, Monster,
    PathfinderMob, SharedEntity, SpawnRule, sync_dirty_mob_effects,
};
use crate::physics::MoveResult;
use crate::world::{LevelReader, World};

const DEFAULT_STEP_HEIGHT: f32 = 0.6;
const XP_REWARD: i32 = 5;

/// A hostile spider entity.
#[entity_behavior(class = "Spider", spawn_rule)]
pub struct SpiderEntity {
    base: EntityBase,
    entity_type: EntityTypeRef,
    living_base: LivingEntityBase,
    mob_base: MobBase,
    entity_data: SyncMutex<SpiderEntityData>,
}

// SAFETY: The owner-scoped type key uniquely identifies SpiderEntity.
unsafe impl DowncastType for SpiderEntity {
    const TYPE_KEY: DowncastTypeKey = DowncastTypeKey::new("steel:entity/spider");
}

impl SpiderEntity {
    /// Vanilla `Spider.checkSpiderSpawnRules` placeholder: monsters may spawn
    /// only when the spawn reason allows hostile natural spawning.
    pub(crate) const SPAWN_RULE: SpawnRule = Self::check_spider_spawn_rules;

    /// Checks the Spider placement predicate.
    pub(crate) fn check_spider_spawn_rules(
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

    /// Creates a new spider entity instance.
    #[must_use]
    pub fn new(entity_type: EntityTypeRef, id: i32, position: DVec3, world: Weak<World>) -> Self {
        Self::new_with_base(
            EntityBase::new(id, position, entity_type.dimensions, world),
            entity_type,
        )
    }

    /// Loads a saved spider entity.
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
        let mut entity_data = SpiderEntityData::new();
        living_base.initialize_synced_data(&mut entity_data);

        {
            let mut goal_selector = mob_base.goal_selector().lock();
            goal_selector.add_goal(0, FloatGoal::new(&mob_base));
            goal_selector.add_goal(2, LeapAtTargetGoal::new(0.4));
            goal_selector.add_goal(4, MeleeAttackGoal::new(1.0, true));
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

impl DefaultEntityPush for SpiderEntity {}

impl Entity for SpiderEntity {
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
        self.play_sound(&sound_events::ENTITY_SPIDER_STEP, 0.15, 1.0);
    }

    fn save_additional(&self, nbt: &mut NbtCompound) {
        self.save_mob(nbt);
    }

    fn load_additional(&self, nbt: BorrowedNbtCompoundView<'_, '_>) {
        self.load_mob(nbt);
    }
}

impl LivingEntity for SpiderEntity {
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
        Some(&sound_events::ENTITY_SPIDER_HURT)
    }

    fn death_sound(&self) -> Option<SoundEventRef> {
        Some(&sound_events::ENTITY_SPIDER_DEATH)
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

impl Mob for SpiderEntity {
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
        Some(&sound_events::ENTITY_SPIDER_AMBIENT)
    }

    fn mob_flags(&self) -> i8 {
        *self.entity_data.lock().mob().mob_flags.get()
    }

    fn set_mob_flags(&self, flags: i8) {
        self.entity_data.lock().mob_mut().mob_flags.set(flags);
    }
}

impl Monster for SpiderEntity {}

impl PathfinderMob for SpiderEntity {}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use glam::DVec3;
    use steel_registry::blocks::block_state_ext::BlockStateExt as _;
    use steel_registry::{init_vanilla_registry, vanilla_blocks, vanilla_entities};
    use steel_utils::types::UpdateFlags;
    use steel_utils::{BlockPos, ChunkPos, Downcast as _};

    use super::SpiderEntity;
    use crate::entity::{Entity, LivingEntity, PathfinderMob, SharedEntity, next_entity_id};
    use crate::test_support::{TestPlayerBuilder, fresh_test_world, insert_ready_full_chunk, tick_test_world};

    #[test]
    fn spider_pathfinds_across_a_flat_platform() {
        init_vanilla_registry();
        crate::behavior::init_behaviors();
        crate::block_entity::init_block_entities();
        let world_fixture = fresh_test_world("spider_pathfind");
        let world = &world_fixture.world;
        insert_ready_full_chunk(world, ChunkPos::new(0, 0));
        for x in 0..32 {
            for z in 0..16 {
                world.set_block(
                    BlockPos::new(x, 64, z),
                    vanilla_blocks::STONE.default_state(),
                    UpdateFlags::UPDATE_CLIENTS,
                );
            }
        }

        let player = TestPlayerBuilder::new(Arc::clone(world), "SpiderAnchor", 7).build();
        assert!(world.add_player(player, crate::player::ResetReason::InitialJoin));
        let spider: SharedEntity = Arc::new(SpiderEntity::new(
            &vanilla_entities::SPIDER,
            next_entity_id(),
            DVec3::new(8.5, 65.0, 8.5),
            Arc::downgrade(world),
        ));
        world
            .try_add_entity(Arc::clone(&spider))
            .expect("spider should attach to the loaded chunk");
        let spider_entity = spider
            .downcast_ref::<SpiderEntity>()
            .expect("spider is concrete");
        for _ in 0..10 {
            spider_entity.ai_step(&spider);
            spider_entity.base_tick();
        }
        assert!(
            spider_entity.on_ground(),
            "spider must settle on the platform first: pos={:?}",
            spider.position()
        );
        assert!(
            world.get_block_state(BlockPos::new(14, 64, 8)).get_block()
                == &vanilla_blocks::STONE,
            "platform block must be visible to the world"
        );
        assert!(
            spider_entity
                .create_path_to(BlockPos::new(14, 65, 8), 1)
                .is_some(),
            "flat platform must produce a direct path"
        );
        assert!(
            spider_entity.move_to_pos(DVec3::new(14.5, 65.0, 8.5), 2.0),
            "flat platform must accept the path target"
        );

        let start_x = spider.position().x;
        for _ in 0..120 {
            spider_entity.ai_step(&spider);
            spider_entity.base_tick();
        }
        let end_x = spider.position().x;
        assert!(
            end_x > start_x + 2.0,
            "spider should walk toward the target: start={start_x}, end={end_x}"
        );
        assert!(
            (end_x - 14.5).abs() < 4.0,
            "spider should approach the path target, got x={end_x}"
        );
    }

    #[test]
    fn spider_entity_type_is_registered_and_spawns_into_world() {
        init_vanilla_registry();
        let world_fixture = fresh_test_world("spider_spawn");
        let world = &world_fixture.world;
        insert_ready_full_chunk(world, ChunkPos::new(0, 0));

        let player = TestPlayerBuilder::new(Arc::clone(world), "SpiderAnchor", 7).build();
        assert!(world.add_player(player, crate::player::ResetReason::InitialJoin));
        let spider: SharedEntity = Arc::new(SpiderEntity::new(
            &vanilla_entities::SPIDER,
            next_entity_id(),
            DVec3::new(8.0, 65.0, 8.0),
            Arc::downgrade(world),
        ));
        world
            .try_add_entity(Arc::clone(&spider))
            .expect("spider should attach to the loaded chunk");
        assert_eq!(spider.entity_type().key.to_string(), "minecraft:spider");

        let spider_entity = spider;
        let spider = spider_entity
            .downcast_ref::<SpiderEntity>()
            .expect("spider is concrete");
        assert_eq!(spider.get_max_health(), 16.0);
        spider.ai_step(&spider_entity);
        assert!(!spider.is_removed());
    }

}
