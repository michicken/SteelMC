//! Villager brain: job-site memory, POI sensor and walk-to-work behavior.
//!
//! This is the first production consumer of the brain subsystem merged from
//! `Steel-Foundation/SteelMC:brain-3-container`. Vanilla villagers use
//! `JOB_SITE`/`POTENTIAL_JOB_SITE` memories plus a walk target; here the sensor
//! reads the real world POI storage and the behavior walks the villager to the
//! nearest workstation.

use std::sync::Arc;

use glam::DVec3;
use steel_registry::villager_profession::VillagerProfessionRef;
use steel_registry::{
    REGISTRY, RegistryExt as _, vanilla_activities, vanilla_villager_professions,
};
use steel_utils::locks::SyncMutex;
use steel_utils::{BlockPos, DowncastType, DowncastTypeKey, Identifier};

use crate::entity::ai::brain::behavior::{Behavior, BehaviorDuration, BehaviorExt};
use crate::entity::ai::brain::container::{ActivityData, Brain};
use crate::entity::ai::brain::context::BrainContext;
use crate::entity::ai::brain::memory::{
    MemoryModuleType, MemoryModuleTypeRef, MemoryStatus, MemoryValue,
};
use crate::entity::ai::brain::sensor::Sensor;
use crate::entity::village::harvest_mature_wheat_near;
use crate::poi::OccupationStatus;

/// A villager's claimed workstation position.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct JobSite(pub BlockPos);

// SAFETY: The owner-scoped type key uniquely identifies JobSite.
unsafe impl DowncastType for JobSite {
    const TYPE_KEY: DowncastTypeKey = DowncastTypeKey::new("steel:villager/job_site");
}

impl MemoryValue for JobSite {}

/// Vanilla villagers keep their workstation in `JOB_SITE`.
pub(crate) static JOB_SITE: MemoryModuleType<JobSite> =
    MemoryModuleType::new(Identifier::new_static("steel", "villager_job_site"));

fn is_workstation(id: usize) -> bool {
    REGISTRY.poi_types.by_id(id).is_some_and(|poi| {
        matches!(
            poi.key.path.as_ref(),
            "armorer"
                | "butcher"
                | "cartographer"
                | "cleric"
                | "farmer"
                | "fisherman"
                | "fletcher"
                | "leatherworker"
                | "librarian"
                | "mason"
                | "shepherd"
                | "toolsmith"
                | "weaponsmith"
        )
    })
}

/// Maps a workstation POI type ID to the vanilla profession it grants.
///
/// Vanilla `VillagerProfession` is tied to the POI type: a composter makes a
/// farmer, a lectern a librarian, and so on.  The mapping lives here so both
/// the POI sensor and the entity's profession assignment share one source of
/// truth.
#[must_use]
pub fn profession_for_workstation_poi(id: usize) -> Option<VillagerProfessionRef> {
    let path = REGISTRY.poi_types.by_id(id)?.key.path.as_ref();
    Some(match path {
        "armorer" => &vanilla_villager_professions::ARMORER,
        "butcher" => &vanilla_villager_professions::BUTCHER,
        "cartographer" => &vanilla_villager_professions::CARTOGRAPHER,
        "cleric" => &vanilla_villager_professions::CLERIC,
        "farmer" => &vanilla_villager_professions::FARMER,
        "fisherman" => &vanilla_villager_professions::FISHERMAN,
        "fletcher" => &vanilla_villager_professions::FLETCHER,
        "leatherworker" => &vanilla_villager_professions::LEATHERWORKER,
        "librarian" => &vanilla_villager_professions::LIBRARIAN,
        "mason" => &vanilla_villager_professions::MASON,
        "shepherd" => &vanilla_villager_professions::SHEPHERD,
        "toolsmith" => &vanilla_villager_professions::TOOLSMITH,
        "weaponsmith" => &vanilla_villager_professions::WEAPONSMITH,
        _ => return None,
    })
}

/// Finds the nearest free workstation together with its profession.
#[must_use]
pub fn nearest_workstation_with_profession(
    world: &crate::world::World,
    center: BlockPos,
    radius: i32,
) -> Option<(BlockPos, VillagerProfessionRef)> {
    let storage = world.poi_storage.lock();
    let pois =
        storage.get_in_horizontal_square(&is_workstation, center, radius, OccupationStatus::Free);
    pois.into_iter()
        .filter_map(|(pos, poi_type)| {
            profession_for_workstation_poi(poi_type).map(|profession| (pos, profession))
        })
        .min_by_key(|(pos, _)| {
            let dx = i64::from(pos.x() - center.x());
            let dy = i64::from(pos.y() - center.y());
            let dz = i64::from(pos.z() - center.z());
            dx * dx + dy * dy + dz * dz
        })
}

/// Finds the nearest workstation POI around `center`.
#[must_use]
pub fn nearest_workstation(
    world: &crate::world::World,
    center: BlockPos,
    radius: i32,
) -> Option<BlockPos> {
    nearest_workstation_with_profession(world, center, radius).map(|(pos, _)| pos)
}

/// Writes the nearest free workstation into `JOB_SITE`.
pub struct JobSiteSensor {
    radius: i32,
}

impl JobSiteSensor {
    #[must_use]
    pub const fn new(radius: i32) -> Self {
        Self { radius }
    }
}

impl Sensor for JobSiteSensor {
    fn requires(&self) -> &[MemoryModuleTypeRef] {
        static REQUIRES: [MemoryModuleTypeRef; 1] = [JOB_SITE.entry()];
        &REQUIRES
    }

    fn scan_rate(&self) -> i32 {
        100
    }

    fn do_tick(&mut self, context: &mut BrainContext<'_>) {
        let site = nearest_workstation(context.level, context.mob.block_position(), self.radius);
        context.memories.set_optional(&JOB_SITE, site.map(JobSite));
    }
}

/// Walks to the remembered job site and harvests mature wheat on arrival.
pub struct WorkAtJobSite;

impl Behavior for WorkAtJobSite {
    fn entry_condition(&self) -> &[(MemoryModuleTypeRef, MemoryStatus)] {
        static REQUIRED: [(MemoryModuleTypeRef, MemoryStatus); 1] =
            [(JOB_SITE.entry(), MemoryStatus::ValuePresent)];
        &REQUIRED
    }

    fn duration(&self) -> BehaviorDuration {
        BehaviorDuration::range(200, 300)
    }

    fn can_still_use(&mut self, _context: &mut BrainContext<'_>) -> bool {
        true
    }

    fn start(&mut self, context: &mut BrainContext<'_>) {
        self.tick(context);
    }

    fn tick(&mut self, context: &mut BrainContext<'_>) {
        let Some(site) = context.memories.get(&JOB_SITE).copied() else {
            return;
        };
        let center = DVec3::new(
            f64::from(site.0.x()) + 0.5,
            f64::from(site.0.y()) + 0.5,
            f64::from(site.0.z()) + 0.5,
        );
        let distance_sq = context.mob.position().distance_squared(center);
        if distance_sq > 4.0 {
            context.mob.move_to_pos(center, 0.5);
            return;
        }

        let world = Arc::clone(context.level);
        // Only farmers farm.  Other professions still walk to their claimed
        // workstation, which is vanilla's `WorkAtPoi`; harvesting wheat for an
        // armorer was a placeholder behavior and is not vanilla.
        let is_farmer = world
            .poi_storage
            .lock()
            .get_type(site.0)
            .and_then(profession_for_workstation_poi)
            .is_some_and(|profession| profession.key.path.as_ref() == "farmer");
        if !is_farmer {
            return;
        }

        let (harvested, replanted) = harvest_mature_wheat_near(&world, site.0, 3);
        if harvested > 0 && replanted > 0 {
            log::debug!(
                "villager harvested {harvested} wheat and replanted {replanted} at {:?}",
                site.0
            );
        }
    }
}

/// Builds the villager's production brain.
#[must_use]
pub fn new_villager_brain() -> SyncMutex<Brain> {
    let mut brain = Brain::new();
    brain.add_sensor(Box::new(JobSiteSensor::new(16)));
    brain.add_activity(ActivityData::new(
        &vanilla_activities::WORK,
        1,
        vec![WorkAtJobSite.control()],
    ));
    brain.set_default_activity(&vanilla_activities::WORK);
    brain.use_default_activity();
    SyncMutex::new(brain)
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use glam::DVec3;
    use steel_registry::{
        RegistryExt as _, init_vanilla_registry, vanilla_blocks, vanilla_entities,
    };
    use steel_utils::{BlockPos, ChunkPos, Downcast as _, types::UpdateFlags};

    use super::{JOB_SITE, nearest_workstation};
    use crate::entity::entities::VillagerEntity;
    use crate::entity::{LivingEntity, PathfinderMob, SharedEntity, next_entity_id};
    use crate::test_support::{fresh_test_world, insert_ready_full_chunk};

    #[test]
    fn villager_brain_sensor_claims_a_composter_job_site() {
        init_vanilla_registry();
        crate::behavior::init_behaviors();
        let fixture = fresh_test_world("villager_brain_job_site");
        let world = &fixture.world;
        insert_ready_full_chunk(world, ChunkPos::new(0, 0));

        let composter = BlockPos::new(12, 64, 8);
        world.set_block(
            composter,
            vanilla_blocks::COMPOSTER.default_state(),
            UpdateFlags::UPDATE_ALL,
        );
        assert_eq!(
            nearest_workstation(world, BlockPos::new(8, 64, 8), 16),
            Some(composter),
            "POI storage must expose the composter as a farmer workstation"
        );

        let villager: SharedEntity = Arc::new(VillagerEntity::new(
            &vanilla_entities::VILLAGER,
            next_entity_id(),
            DVec3::new(8.5, 64.0, 8.5),
            Arc::downgrade(world),
        ));
        world
            .try_add_entity(Arc::clone(&villager))
            .expect("villager must attach");
        let concrete = villager
            .downcast_ref::<VillagerEntity>()
            .expect("villager must be concrete");
        let mob: &dyn PathfinderMob = villager
            .as_pathfinder_mob()
            .expect("villager is pathfinder");

        let mut brain = concrete.brain.lock();
        for tick in 0..150 {
            brain.tick(mob, world, i64::from(tick));
        }
        assert_eq!(
            brain.memories().get(&JOB_SITE).map(|site| site.0),
            Some(composter),
            "villager brain must remember the nearest workstation"
        );
    }

    #[test]
    fn villager_ai_claims_workstation_and_sets_matching_profession() {
        use steel_registry::vanilla_villager_professions;

        init_vanilla_registry();
        crate::behavior::init_behaviors();
        let fixture = fresh_test_world("villager_profession_claim");
        let world = &fixture.world;
        insert_ready_full_chunk(world, ChunkPos::new(0, 0));

        let composter = BlockPos::new(12, 64, 8);
        world.set_block(
            composter,
            vanilla_blocks::COMPOSTER.default_state(),
            UpdateFlags::UPDATE_ALL,
        );

        let villager: SharedEntity = Arc::new(VillagerEntity::new(
            &vanilla_entities::VILLAGER,
            next_entity_id(),
            DVec3::new(8.5, 64.0, 8.5),
            Arc::downgrade(world),
        ));
        world
            .try_add_entity(Arc::clone(&villager))
            .expect("villager must attach");
        let concrete = villager
            .downcast_ref::<VillagerEntity>()
            .expect("villager must be concrete");

        assert_eq!(
            concrete.profession_id(),
            0,
            "fresh villagers start unemployed"
        );
        for _ in 0..260 {
            concrete.ai_step(&villager);
        }

        let farmer_id = steel_registry::REGISTRY
            .villager_professions
            .id_from_key(&vanilla_villager_professions::FARMER.key)
            .and_then(|id| i32::try_from(id).ok())
            .expect("farmer profession is registered");
        assert_eq!(
            concrete.profession_id(),
            farmer_id,
            "a composter must turn the villager into a farmer"
        );
        assert_eq!(concrete.villager_level(), 1);
        assert_eq!(concrete.claimed_job_site(), Some(composter));
        assert!(
            world.poi_storage.lock().is_occupied(composter),
            "the claimed workstation must hold a reserved POI ticket"
        );
        assert_eq!(
            concrete.trade_offers().len(),
            5,
            "becoming a farmer must install the vanilla level-1 offer table"
        );

        use crate::inventory::container::Container as _;
        use crate::test_support::TestPlayerBuilder;
        let player = TestPlayerBuilder::new(Arc::clone(world), "TradeCustomer", 1).build();
        player.inventory.lock().set_item(
            0,
            steel_registry::item_stack::ItemStack::with_count(
                &steel_registry::vanilla_items::WHEAT,
                20,
            ),
        );
        let outcome = concrete
            .trade_with(&player, 4)
            .expect("farmer must sell an emerald for 20 wheat");
        assert_eq!(outcome.xp, 2);
        assert_eq!(
            concrete.villager_level(),
            2,
            "first trade levels the villager to 2"
        );
        let inventory = player.inventory.lock();
        assert!(
            inventory
                .items()
                .iter()
                .any(|stack| stack.item() == &*steel_registry::vanilla_items::EMERALD),
            "the trade must insert a real emerald into the player inventory"
        );
    }
}
