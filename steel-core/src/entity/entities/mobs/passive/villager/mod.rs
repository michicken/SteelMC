use std::sync::Weak;

mod brain;
mod merchant;
mod trade_table;

pub use merchant::{MerchantOffer, TradeError, TradeOutcome, level_for_xp};

use glam::DVec3;
use rand::Rng;
use simdnbt::borrow::NbtCompound as BorrowedNbtCompoundView;
use simdnbt::owned::NbtCompound;
use steel_macros::entity_behavior;
use steel_protocol::packets::game::SoundSource;
use steel_registry::entity_type::{EntityDimensions, EntityTypeRef};
use steel_registry::sound_event::SoundEventRef;
use steel_registry::vanilla_entity_data::VillagerEntityData;
use steel_registry::{REGISTRY, RegistryExt as _, sound_events, vanilla_attributes};
use steel_utils::locks::SyncMutex;
use steel_utils::nbt::NbtValueInput as _;
use steel_utils::{BlockPos, BlockStateId, DowncastType, DowncastTypeKey};

use crate::entity::ai::goal::{
    FloatGoal, LookAtPlayerGoal, RandomLookAroundGoal, WaterAvoidingRandomStrollGoal,
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

/// A passive villager entity.
#[entity_behavior(class = "Villager", spawn_rule)]
pub struct VillagerEntity {
    base: EntityBase,
    entity_type: EntityTypeRef,
    living_base: LivingEntityBase,
    mob_base: MobBase,
    entity_data: SyncMutex<VillagerEntityData>,
    village_ticks: SyncMutex<i32>,
    brain: SyncMutex<crate::entity::ai::brain::container::Brain>,
    /// The workstation this villager has claimed, if any.  Vanilla keeps the
    /// claim as a POI ticket in addition to the `JOB_SITE` memory.
    claimed_job_site: SyncMutex<Option<BlockPos>>,
    /// Current merchant offers.  Vanilla generates a fresh set when the
    /// profession is acquired (and on level-up).
    offers: SyncMutex<Vec<MerchantOffer>>,
    /// Cumulative trade XP used for profession level-ups.
    trade_xp: SyncMutex<i32>,
}

// SAFETY: The owner-scoped type key uniquely identifies VillagerEntity.
unsafe impl DowncastType for VillagerEntity {
    const TYPE_KEY: DowncastTypeKey = DowncastTypeKey::new("steel:entity/villager");
}

impl VillagerEntity {
    /// Villager spawn rule: allowed for commands/spawners; natural village spawns are
    /// handled by the village manager.
    pub(crate) const SPAWN_RULE: SpawnRule = |_entity_type, _level, spawn_reason, _pos, _random| {
        spawn_reason.is_spawner() || matches!(spawn_reason, EntitySpawnReason::Command)
    };

    /// Creates a new villager entity instance.
    #[must_use]
    pub fn new(entity_type: EntityTypeRef, id: i32, position: DVec3, world: Weak<World>) -> Self {
        Self::new_with_base(
            EntityBase::new(id, position, entity_type.dimensions, world),
            entity_type,
        )
    }

    /// Loads a saved villager entity.
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
        let mut entity_data = VillagerEntityData::new();
        living_base.initialize_synced_data(&mut entity_data);

        {
            let mut goal_selector = mob_base.goal_selector().lock();
            goal_selector.add_goal(1, FloatGoal::new(&mob_base));
            goal_selector.add_goal(3, WaterAvoidingRandomStrollGoal::new(0.6));
            goal_selector.add_goal(7, LookAtPlayerGoal::new(8.0));
            goal_selector.add_goal(8, RandomLookAroundGoal::new());
        }

        Self {
            base,
            entity_type,
            living_base,
            mob_base,
            entity_data: SyncMutex::new(entity_data),
            village_ticks: SyncMutex::new(0),
            brain: brain::new_villager_brain(),
            claimed_job_site: SyncMutex::new(None),
            offers: SyncMutex::new(Vec::new()),
            trade_xp: SyncMutex::new(0),
        }
    }

    fn update_dirty_mob_effect_entity_data(&self) {
        sync_dirty_mob_effects(&self.living_base, &self.entity_data);
    }

    /// Returns the villager's current profession registry ID.
    #[must_use]
    pub fn profession_id(&self) -> i32 {
        self.entity_data.lock().villager_data.get().profession
    }

    /// Returns the villager's current trading level (1-5).
    #[must_use]
    pub fn villager_level(&self) -> i32 {
        self.entity_data.lock().villager_data.get().level
    }

    /// Returns the claimed workstation position.
    #[must_use]
    pub fn claimed_job_site(&self) -> Option<BlockPos> {
        *self.claimed_job_site.lock()
    }

    /// Returns a snapshot of the current offers.
    #[must_use]
    pub fn trade_offers(&self) -> Vec<MerchantOffer> {
        self.offers.lock().clone()
    }

    /// Rebuilds the offer list from the vanilla table for `profession_id`.
    fn refresh_trade_offers(&self, profession_id: i32) {
        let path = usize::try_from(profession_id)
            .ok()
            .and_then(|id| REGISTRY.villager_professions.by_id(id))
            .map(|profession| profession.key.path.to_string());
        let level = self.villager_level();
        let offers = path
            .as_deref()
            .map_or_else(Vec::new, |path| merchant::offers_for_level(path, level));
        *self.offers.lock() = offers;
    }

    /// Executes one merchant offer against a real player inventory.
    ///
    /// This is the same settlement path a merchant menu uses: it validates the
    /// wanted stacks, removes them, inserts the result and advances trade XP.
    pub fn trade_with(
        &self,
        player: &crate::player::Player,
        index: usize,
    ) -> Result<TradeOutcome, TradeError> {
        let before_level = self.villager_level();
        let mut offers = self.offers.lock();
        let mut outcome = merchant::execute_trade(&mut offers, player, index)?;
        drop(offers);

        self.add_trade_xp(outcome.xp);
        outcome.level_ups = self.villager_level() - before_level;
        Ok(outcome)
    }

    fn add_trade_xp(&self, amount: i32) {
        if amount == 0 {
            return;
        }
        let mut xp = self.trade_xp.lock();
        *xp = xp.saturating_add(amount);
        let level = merchant::level_for_xp(*xp);
        let mut data = self.entity_data.lock();
        let mut villager_data = data.villager_data.get().clone();
        if villager_data.level != level {
            villager_data.level = level;
            let profession = villager_data.profession;
            data.villager_data.set(villager_data);
            drop(data);
            self.refresh_trade_offers(profession);
        }
    }

    /// Sets the synced profession.  Vanilla also clears the workstation claim
    /// when the profession changes, so a villager cannot keep a job site that
    /// belongs to another profession.
    pub fn set_profession(&self, profession_id: i32) {
        let mut data = self.entity_data.lock();
        let mut villager_data = data.villager_data.get().clone();
        if villager_data.profession == profession_id {
            drop(data);
            if self.offers.lock().is_empty() {
                self.refresh_trade_offers(profession_id);
            }
            return;
        }
        villager_data.profession = profession_id;
        villager_data.level = 1;
        data.villager_data.set(villager_data);
        drop(data);
        *self.trade_xp.lock() = 0;
        self.refresh_trade_offers(profession_id);
    }

    /// Assigns the profession implied by a workstation POI and reserves its
    /// ticket, mirroring vanilla `Villager.acquirePoi`.
    ///
    /// Returns `true` when the villager newly claimed the site.
    pub fn claim_job_site(&self, world: &World, site: BlockPos) -> bool {
        let Some(poi_type) = world.poi_storage.lock().get_type(site) else {
            return false;
        };
        let Some(profession) = brain::profession_for_workstation_poi(poi_type) else {
            return false;
        };
        let Some(profession_id) = REGISTRY
            .villager_professions
            .id_from_key(&profession.key)
            .and_then(|id| i32::try_from(id).ok())
        else {
            return false;
        };

        let mut claimed = self.claimed_job_site.lock();
        if *claimed == Some(site) {
            // Already claimed: make sure the synced profession still matches
            // (a reloaded entity can lose the data value before the claim).
            self.set_profession(profession_id);
            return false;
        }

        if !world.poi_storage.lock().reserve_ticket(site) {
            return false;
        }
        if let Some(old) = *claimed {
            let _ = world.poi_storage.lock().release_ticket(old);
        }
        *claimed = Some(site);
        self.set_profession(profession_id);
        true
    }

    /// Releases the workstation claim, e.g. when a POI is broken or the
    /// villager is removed.
    pub fn release_job_site(&self, world: &World) {
        if let Some(site) = self.claimed_job_site.lock().take() {
            let _ = world.poi_storage.lock().release_ticket(site);
        }
    }
}

impl DefaultEntityPush for VillagerEntity {}

impl Entity for VillagerEntity {
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
        self.play_sound(&sound_events::ENTITY_VILLAGER_AMBIENT, 0.15, 1.0);
    }

    fn save_additional(&self, nbt: &mut NbtCompound) {
        self.save_mob(nbt);
    }

    fn load_additional(&self, nbt: BorrowedNbtCompoundView<'_, '_>) {
        self.load_mob(nbt);
    }
}

impl LivingEntity for VillagerEntity {
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
        Some(&sound_events::ENTITY_VILLAGER_HURT)
    }

    fn death_sound(&self) -> Option<SoundEventRef> {
        Some(&sound_events::ENTITY_VILLAGER_DEATH)
    }

    fn server_ai_step(&self, entity: &SharedEntity) {
        Mob::mob_server_ai_step(self, entity);
    }

    fn ai_step(&self, entity: &SharedEntity) -> Option<MoveResult> {
        let result = Mob::mob_ai_step(self, entity);
        if let Some(world) = self.level()
            && let Some(mob) = entity.as_pathfinder_mob()
        {
            let time = world.game_time();
            self.brain.lock().tick(mob, &world, time);

            // Convert the brain's remembered workstation into a real POI claim
            // and a synced profession, exactly like vanilla's job-site sensor.
            let remembered = self.brain.lock().memories().get(&brain::JOB_SITE).copied();
            if let Some(site) = remembered {
                self.claim_job_site(&world, site.0);
            }
        }
        let due = {
            let mut ticks = self.village_ticks.lock();
            *ticks = ticks.wrapping_add(1);
            *ticks % 200 == 0
        };
        if due && let Some(world) = self.level() {
            let center = self.block_position();
            crate::entity::village::tick_village_crop_farm(&world, center);
            crate::entity::village::try_spawn_iron_golem(&world, center);
        }
        result
    }
}

impl Mob for VillagerEntity {
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
        Some(&sound_events::ENTITY_VILLAGER_AMBIENT)
    }

    fn mob_flags(&self) -> i8 {
        *self.entity_data.lock().mob().mob_flags.get()
    }

    fn set_mob_flags(&self, flags: i8) {
        self.entity_data.lock().mob_mut().mob_flags.set(flags);
    }
}

impl PathfinderMob for VillagerEntity {}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use glam::DVec3;
    use steel_registry::{init_vanilla_registry, vanilla_attributes, vanilla_entities};
    use steel_utils::{ChunkPos, Downcast as _};

    use super::VillagerEntity;
    use crate::entity::{Entity, LivingEntity, Mob, SharedEntity, next_entity_id};
    use crate::test_support::{fresh_test_world, insert_ready_full_chunk};

    #[test]
    fn villager_spawns_with_vanilla_health_and_stays_passive() {
        init_vanilla_registry();
        let fixture = fresh_test_world("villager_spawn");
        let world = &fixture.world;
        insert_ready_full_chunk(world, ChunkPos::new(0, 0));
        let villager: SharedEntity = Arc::new(VillagerEntity::new(
            &vanilla_entities::VILLAGER,
            next_entity_id(),
            DVec3::new(8.0, 64.0, 8.0),
            Arc::downgrade(world),
        ));
        world
            .try_add_entity(Arc::clone(&villager))
            .expect("villager must attach to loaded chunk");
        let concrete = villager
            .downcast_ref::<VillagerEntity>()
            .expect("villager must be concrete");
        assert_eq!(concrete.entity_type().key.path.as_ref(), "villager");
        assert_eq!(concrete.get_health(), 20.0);
        assert_eq!(
            concrete
                .attributes()
                .lock()
                .get_value(vanilla_attributes::MAX_HEALTH),
            Some(20.0)
        );
        assert!(concrete.get_health() > 0.0);
    }
}
