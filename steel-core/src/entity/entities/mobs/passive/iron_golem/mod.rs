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
use steel_registry::item_stack::ItemStack;
use steel_registry::vanilla_entity_data::IronGolemEntityData;
use steel_registry::{sound_events, vanilla_attributes, vanilla_items};
use steel_utils::locks::SyncMutex;
use steel_utils::nbt::NbtValueInput as _;
use steel_utils::entity_events::EntityStatus;
use steel_utils::{BlockPos, BlockStateId, DowncastType, DowncastTypeKey};

use crate::entity::ai::goal::{
    FloatGoal, HurtByTargetGoal, LookAtPlayerGoal, MeleeAttackGoal,
    NearestAttackableTargetGoal, RandomLookAroundGoal, WaterAvoidingRandomStrollGoal,
};
use crate::entity::damage::DamageSource;
use crate::entity::{
    DefaultEntityPush, Entity, EntityBase, EntityBaseLoad, EntityMovementEmission, EntityPose,
    EntitySpawnReason, EntitySyncedData, LivingEntity, LivingEntityBase, Mob, MobBase,
    PathfinderMob, SharedEntity, SpawnRule, sync_dirty_mob_effects,
};
use crate::physics::MoveResult;
use crate::world::{LevelReader, World};

const DEFAULT_STEP_HEIGHT: f32 = 0.6;
const XP_REWARD: i32 = 0;

/// A passive iron_golem entity.
#[entity_behavior(class = "IronGolem", spawn_rule)]
pub struct IronGolemEntity {
    base: EntityBase,
    entity_type: EntityTypeRef,
    living_base: LivingEntityBase,
    mob_base: MobBase,
    entity_data: SyncMutex<IronGolemEntityData>,
}

// SAFETY: The owner-scoped type key uniquely identifies IronGolemEntity.
unsafe impl DowncastType for IronGolemEntity {
    const TYPE_KEY: DowncastTypeKey = DowncastTypeKey::new("steel:entity/iron_golem");
}

impl IronGolemEntity {
    /// Iron golem spawn rule: village manager is the only natural source.
    pub(crate) const SPAWN_RULE: SpawnRule = |_entity_type, _level, spawn_reason, _pos, _random| {
        spawn_reason.is_spawner()
    };

    /// Creates a new iron_golem entity instance.
    #[must_use]
    pub fn new(entity_type: EntityTypeRef, id: i32, position: DVec3, world: Weak<World>) -> Self {
        Self::new_with_base(
            EntityBase::new(id, position, entity_type.dimensions, world),
            entity_type,
        )
    }

    /// Loads a saved iron_golem entity.
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
        let mut entity_data = IronGolemEntityData::new();
        living_base.initialize_synced_data(&mut entity_data);

        {
            let mut goal_selector = mob_base.goal_selector().lock();
            goal_selector.add_goal(1, FloatGoal::new(&mob_base));
            goal_selector.add_goal(2, MeleeAttackGoal::new(1.0, false));
            goal_selector.add_goal(3, WaterAvoidingRandomStrollGoal::new(0.5));
            goal_selector.add_goal(7, LookAtPlayerGoal::new(8.0));
            goal_selector.add_goal(8, RandomLookAroundGoal::new());

            let mut target_selector = mob_base.target_selector().lock();
            // Retaliate against whatever hurt the golem; do not alert other
            // golems in this build.
            target_selector.add_goal(1, HurtByTargetGoal::new().set_alert_others(empty()));
            // Patrol golems hunt nearby monsters; players stay neutral unless
            // the golem is provoked (handled by the hurt-by-target goal).
            target_selector.add_goal(
                2,
                NearestAttackableTargetGoal::new(true, |target, _| {
                    target.as_monster().is_some()
                }),
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

impl DefaultEntityPush for IronGolemEntity {}

impl Entity for IronGolemEntity {
    fn is_hard_collision_relevant(&self) -> bool {
        false
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
        SoundSource::Neutral
    }

    fn play_step_sound(&self, _pos: BlockPos, _block_state: BlockStateId) {
        self.play_sound(&sound_events::ENTITY_IRON_GOLEM_STEP, 0.15, 1.0);
    }

    fn save_additional(&self, nbt: &mut NbtCompound) {
        self.save_mob(nbt);
    }

    fn load_additional(&self, nbt: BorrowedNbtCompoundView<'_, '_>) {
        self.load_mob(nbt);
    }
}

impl LivingEntity for IronGolemEntity {
    fn drop_custom_death_loot(&self, _source: &DamageSource, _killed_by_player: bool) {
        let Some(world) = self.level() else {
            return;
        };
        let drop_pos = self.position() + DVec3::new(0.0, 0.5, 0.0);
        // Vanilla iron golem: 3-5 iron ingots, plus 0-2 poppies.
        let ingots = rand::random_range(3..=5);
        for _ in 0..ingots {
            world.spawn_item(drop_pos, ItemStack::new(&vanilla_items::IRON_INGOT));
        }
        let poppies = rand::random_range(0..=2);
        for _ in 0..poppies {
            world.spawn_item(drop_pos, ItemStack::new(&vanilla_items::POPPY));
        }
    }

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
        Some(&sound_events::ENTITY_IRON_GOLEM_HURT)
    }

    fn death_sound(&self) -> Option<SoundEventRef> {
        Some(&sound_events::ENTITY_IRON_GOLEM_DEATH)
    }


    fn server_ai_step(&self, entity: &SharedEntity) {
        Mob::mob_server_ai_step(self, entity);
    }

    fn ai_step(&self, entity: &SharedEntity) -> Option<MoveResult> {
        Mob::mob_ai_step(self, entity)
    }
}

impl Mob for IronGolemEntity {
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
        Some(&sound_events::ENTITY_IRON_GOLEM_STEP)
    }

    fn mob_flags(&self) -> i8 {
        *self.entity_data.lock().mob().mob_flags.get()
    }

    fn set_mob_flags(&self, flags: i8) {
        self.entity_data.lock().mob_mut().mob_flags.set(flags);
    }

    /// Vanilla `IronGolem.doHurtTarget` sets the 10-tick attack animation and
    /// broadcasts entity event 4. The client golem model does not animate its
    /// arms from `CSwingAnimation` alone, so without this event an attack deals
    /// damage but looks static.
    fn do_hurt_target(&self, entity: &SharedEntity, world: &World, target: &SharedEntity) -> bool {
        let hurt = crate::entity::mob::do_hurt_target_default(self, entity, world, target);
        if hurt {
            self.broadcast_entity_event(EntityStatus::StartAttacking);
        }
        hurt
    }
}

impl PathfinderMob for IronGolemEntity {}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use glam::DVec3;
    use steel_registry::{
        init_vanilla_registry, vanilla_blocks, vanilla_damage_types, vanilla_entities,
    };
    use steel_utils::{BlockPos, ChunkPos, Downcast as _};

    use super::IronGolemEntity;
    use crate::entity::entities::{PigEntity, ZombieEntity};
    use crate::entity::damage::DamageSource;
    use crate::entity::{Entity, LivingEntity, Mob, SharedEntity, next_entity_id};
    use crate::test_support::{fresh_test_world, insert_ready_full_chunk, tick_test_world};

    fn flat_platform(world: &crate::test_support::TestWorld) {
        insert_ready_full_chunk(&world.world, ChunkPos::new(0, 0));
        for x in 0..20 {
            for z in 0..16 {
                world.world.set_block(
                    BlockPos::new(x, 64, z),
                    vanilla_blocks::STONE.default_state(),
                    steel_utils::types::UpdateFlags::UPDATE_CLIENTS,
                );
            }
        }
    }

    #[test]
    fn iron_golem_targets_and_chases_nearby_monster() {
        init_vanilla_registry();
        crate::behavior::init_behaviors();
        let world_fixture = fresh_test_world("iron_golem_targets_monster");
        let world = &world_fixture.world;
        flat_platform(&world_fixture);

        let golem: SharedEntity = Arc::new(IronGolemEntity::new(
            &vanilla_entities::IRON_GOLEM,
            next_entity_id(),
            DVec3::new(8.5, 65.0, 8.5),
            Arc::downgrade(world),
        ));
        world
            .try_add_entity(Arc::clone(&golem))
            .expect("golem should attach to the loaded chunk");

        let zombie: SharedEntity = Arc::new(ZombieEntity::new(
            &vanilla_entities::ZOMBIE,
            next_entity_id(),
            DVec3::new(11.5, 65.0, 8.5),
            Arc::downgrade(world),
        ));
        world
            .try_add_entity(Arc::clone(&zombie))
            .expect("zombie should attach to the loaded chunk");

        let golem_entity = golem
            .downcast_ref::<IronGolemEntity>()
            .expect("golem is concrete");

        for _ in 0..40 {
            tick_test_world(world, 1, true);
            golem_entity.ai_step(&golem);
            golem_entity.base_tick();
        }

        let target = golem_entity
            .target()
            .expect("golem must acquire the nearby zombie as a target");
        assert_eq!(target.uuid(), zombie.uuid());
        assert!(target.as_monster().is_some(), "target must be a monster");
    }

    #[test]
    fn iron_golem_retaliates_against_its_attacker() {
        init_vanilla_registry();
        crate::behavior::init_behaviors();
        let world_fixture = fresh_test_world("iron_golem_retaliates");
        let world = &world_fixture.world;
        flat_platform(&world_fixture);

        let golem: SharedEntity = Arc::new(IronGolemEntity::new(
            &vanilla_entities::IRON_GOLEM,
            next_entity_id(),
            DVec3::new(8.5, 65.0, 8.5),
            Arc::downgrade(world),
        ));
        world
            .try_add_entity(Arc::clone(&golem))
            .expect("golem should attach to the loaded chunk");

        // The attacker is a passive pig, which the monster-targeting goal
        // ignores; any retaliation target must come from the hurt-by goal.
        let attacker: SharedEntity = Arc::new(PigEntity::new(
            &vanilla_entities::PIG,
            next_entity_id(),
            DVec3::new(10.0, 65.0, 8.5),
            Arc::downgrade(world),
        ));
        world
            .try_add_entity(Arc::clone(&attacker))
            .expect("attacker should attach to the loaded chunk");

        let golem_entity = golem
            .downcast_ref::<IronGolemEntity>()
            .expect("golem is concrete");

        // Advance the clock before the hit so the recorded attacker timestamp
        // differs from the goal's initial zero timestamp.
        golem_entity.advance_tick_count();
        let source = DamageSource::direct(&vanilla_damage_types::GENERIC, attacker.clone());
        assert!(golem_entity.hurt(world, &source, 4.0));

        let hurt_by = golem_entity.last_hurt_by_mob();
        assert!(hurt_by.is_some(), "golem must remember its attacker");

        // The golem must acquire its attacker as a retaliation target on the
        // next AI steps. The attacker here is a low-HP passive pig; the
        // assertion only covers acquisition, not retention after the
        // retaliation exchange (a dead target is correctly dropped later).
        for _ in 0..5 {
            tick_test_world(world, 1, true);
            golem_entity.ai_step(&golem);
            golem_entity.base_tick();
        }

        let target = golem_entity
            .target()
            .expect("golem must retaliate against its attacker");
        assert_eq!(target.uuid(), attacker.uuid());
    }
}
