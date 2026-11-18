//! Villager brain: job-site memory, POI sensor and walk-to-work behavior.
//!
//! This is the first production consumer of the brain subsystem merged from
//! `Steel-Foundation/SteelMC:brain-3-container`. Vanilla villagers use
//! `JOB_SITE`/`POTENTIAL_JOB_SITE` memories plus a walk target; here the sensor
//! reads the real world POI storage and the behavior walks the villager to the
//! nearest workstation.

use std::sync::Arc;

use glam::DVec3;
use steel_registry::{REGISTRY, RegistryExt as _, vanilla_activities};
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
pub static JOB_SITE: MemoryModuleType<JobSite> =
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

/// Finds the nearest workstation POI around `center`.
#[must_use]
pub fn nearest_workstation(
    world: &crate::world::World,
    center: BlockPos,
    radius: i32,
) -> Option<BlockPos> {
    let storage = world.poi_storage.lock();
    let pois =
        storage.get_in_horizontal_square(&is_workstation, center, radius, OccupationStatus::Free);
    pois.into_iter()
        .min_by_key(|(pos, _)| {
            let dx = i64::from(pos.x() - center.x());
            let dy = i64::from(pos.y() - center.y());
            let dz = i64::from(pos.z() - center.z());
            dx * dx + dy * dy + dz * dz
        })
        .map(|(pos, _)| pos)
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
    use steel_registry::{init_vanilla_registry, vanilla_blocks, vanilla_entities};
    use steel_utils::{BlockPos, ChunkPos, Downcast as _, types::UpdateFlags};

    use super::{JOB_SITE, nearest_workstation};
    use crate::entity::entities::VillagerEntity;
    use crate::entity::{Entity, PathfinderMob, SharedEntity, next_entity_id};
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
}
