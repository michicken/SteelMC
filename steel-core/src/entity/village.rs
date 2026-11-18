//! Minimal village POI manager: beds + workstations + villagers drive iron golem spawning.
//!
//! This is deliberately data-driven rather than tied to a structure: it scans loaded
//! blocks around a village centre, counts valid POIs, and spawns an iron golem when the
//! vanilla population/bed/workstation thresholds are met.

use std::sync::Arc;

use glam::DVec3;
use steel_registry::blocks::block_state_ext::BlockStateExt as _;
use steel_registry::blocks::properties::BlockStateProperties;
use steel_registry::item_stack::ItemStack;
use steel_registry::{vanilla_blocks, vanilla_entities, vanilla_items};
use steel_utils::{BlockPos, Downcast as _, WorldAabb};

use crate::entity::entities::IronGolemEntity;
use crate::entity::{Entity, SharedEntity, next_entity_id};
use crate::world::World;

/// Radius (in blocks) scanned for beds and workstations around a village centre.
pub const VILLAGE_POI_RADIUS: i32 = 16;
/// Minimum villagers required for an iron golem attempt.
pub const MIN_VILLAGERS: usize = 3;
/// Minimum beds required for an iron golem attempt.
pub const MIN_BEDS: usize = 3;
/// Minimum workstations required for an iron golem attempt.
pub const MIN_WORKSTATIONS: usize = 3;

/// Counts beds and workstations in a horizontal band around `center`.
///
/// Uses the world's real POI storage (the same store vanilla uses for
/// village metadata) instead of scanning every block in the area.
#[must_use]
pub fn count_village_pois(world: &World, center: BlockPos) -> (usize, usize) {
    use steel_registry::{REGISTRY, RegistryExt as _};

    fn path_of(id: usize) -> &'static str {
        REGISTRY
            .poi_types
            .by_id(id)
            .map_or("", |poi| poi.key.path.as_ref())
    }

    fn is_home(id: usize) -> bool {
        path_of(id) == "home"
    }

    fn is_workstation(id: usize) -> bool {
        matches!(
            path_of(id),
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
    }

    let storage = world.poi_storage.lock();
    let pois = storage.get_in_horizontal_square(
        &|id| is_home(id) || is_workstation(id),
        center,
        VILLAGE_POI_RADIUS,
        crate::poi::OccupationStatus::Any,
    );
    let mut beds = 0;
    let mut workstations = 0;
    for (pos, id) in pois {
        // Village POIs are counted in the same horizontal band the vanilla
        // village manager uses; ignore beds on other floors.
        if (pos.y() - center.y()).abs() > 2 {
            continue;
        }
        if is_home(id) {
            beds += 1;
        } else {
            workstations += 1;
        }
    }
    (beds, workstations)
}

/// Returns villagers within `radius` of `center`.
#[must_use]
pub fn nearby_villagers(world: &World, center: BlockPos, radius: f64) -> Vec<SharedEntity> {
    let aabb = WorldAabb::new(
        f64::from(center.x()) - radius,
        f64::from(center.y()) - radius,
        f64::from(center.z()) - radius,
        f64::from(center.x()) + radius,
        f64::from(center.y()) + radius,
        f64::from(center.z()) + radius,
    );
    world.get_entities_in_aabb_matching(&aabb, |entity| {
        entity.entity_type() == &vanilla_entities::VILLAGER
    })
}

/// Tries to spawn an iron golem for a village at `center`.
///
/// Returns the spawned golem when the population/bed/workstation thresholds pass.
pub fn try_spawn_iron_golem(world: &Arc<World>, center: BlockPos) -> Option<SharedEntity> {
    let golems = world.get_entities_in_aabb_matching(
        &WorldAabb::new(
            f64::from(center.x()) - f64::from(VILLAGE_POI_RADIUS),
            f64::from(center.y()) - f64::from(VILLAGE_POI_RADIUS),
            f64::from(center.z()) - f64::from(VILLAGE_POI_RADIUS),
            f64::from(center.x()) + f64::from(VILLAGE_POI_RADIUS),
            f64::from(center.y()) + f64::from(VILLAGE_POI_RADIUS),
            f64::from(center.z()) + f64::from(VILLAGE_POI_RADIUS),
        ),
        |entity| entity.entity_type() == &vanilla_entities::IRON_GOLEM,
    );
    if !golems.is_empty() {
        return None;
    }
    let villagers = nearby_villagers(world, center, f64::from(VILLAGE_POI_RADIUS));
    let (beds, workstations) = count_village_pois(world, center);
    if villagers.len() < MIN_VILLAGERS || beds < MIN_BEDS || workstations < MIN_WORKSTATIONS {
        return None;
    }

    let spawn = DVec3::new(
        f64::from(center.x()) + 0.5,
        f64::from(center.y()) + 1.0,
        f64::from(center.z()) + 0.5,
    );
    let golem: SharedEntity = Arc::new(IronGolemEntity::new(
        &vanilla_entities::IRON_GOLEM,
        next_entity_id(),
        spawn,
        Arc::downgrade(world),
    ));
    world.try_add_entity(Arc::clone(&golem)).ok()?;
    Some(golem)
}

/// Harvests mature wheat within `radius` of `center` and replants it.
///
/// Returns `(harvested, replanted)` counts. Drops are real item entities so
/// hoppers can collect them.
pub fn harvest_mature_wheat_near(
    world: &Arc<World>,
    center: BlockPos,
    radius: i32,
) -> (usize, usize) {
    let mut harvested = 0;
    let mut replanted = 0;
    for dx in -radius..=radius {
        for dz in -radius..=radius {
            for dy in -1..=1 {
                let pos = center.offset(dx, dy, dz);
                let state = world.get_block_state(pos);
                if state.get_block() != &vanilla_blocks::WHEAT {
                    continue;
                }
                if state.get_value(&BlockStateProperties::AGE_7) < 7 {
                    continue;
                }
                world.set_block(
                    pos,
                    vanilla_blocks::AIR.default_state(),
                    steel_utils::types::UpdateFlags::UPDATE_ALL,
                );
                world.spawn_item(
                    glam::DVec3::new(
                        f64::from(pos.x()) + 0.5,
                        f64::from(pos.y()) + 0.5,
                        f64::from(pos.z()) + 0.5,
                    ),
                    ItemStack::new(&vanilla_items::WHEAT),
                );
                world.spawn_item(
                    glam::DVec3::new(
                        f64::from(pos.x()) + 0.5,
                        f64::from(pos.y()) + 0.5,
                        f64::from(pos.z()) + 0.5,
                    ),
                    ItemStack::new(&vanilla_items::WHEAT_SEEDS),
                );
                world.set_block(
                    pos,
                    vanilla_blocks::WHEAT.default_state(),
                    steel_utils::types::UpdateFlags::UPDATE_ALL,
                );
                harvested += 1;
                replanted += 1;
            }
        }
    }
    (harvested, replanted)
}

/// Harvests mature wheat within four blocks of each village and replants it.
///
/// Returns `(harvested, replanted)` counts. This is the village crop-farm loop:
/// villagers are the workforce, farmland/wheat are the POIs, and drops are real
/// item entities so hoppers can collect them.
pub fn tick_village_crop_farm(world: &Arc<World>, center: BlockPos) -> (usize, usize) {
    let villagers = nearby_villagers(world, center, f64::from(VILLAGE_POI_RADIUS));
    let mut harvested = 0;
    let mut replanted = 0;
    for villager in villagers {
        let (h, r) = harvest_mature_wheat_near(world, villager.block_position(), 4);
        harvested += h;
        replanted += r;
    }
    (harvested, replanted)
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use glam::DVec3;
    use steel_registry::blocks::block_state_ext::BlockStateExt as _;
    use steel_registry::blocks::properties::{BedPart, BlockStateProperties};
    use steel_registry::{init_vanilla_registry, vanilla_blocks, vanilla_entities};
    use steel_utils::types::UpdateFlags;
    use steel_utils::{ChunkPos, Downcast as _};

    use super::{MIN_VILLAGERS, try_spawn_iron_golem};
    use crate::entity::entities::{IronGolemEntity, VillagerEntity};
    use crate::entity::{Entity, LivingEntity, SharedEntity, next_entity_id};
    use crate::test_support::{fresh_test_world, insert_ready_full_chunk};

    fn villager(world: &Arc<crate::world::World>, x: f64) -> SharedEntity {
        Arc::new(VillagerEntity::new(
            &vanilla_entities::VILLAGER,
            next_entity_id(),
            DVec3::new(x, 64.0, 8.0),
            Arc::downgrade(world),
        ))
    }

    #[test]
    fn village_with_three_villagers_beds_and_composters_spawns_iron_golem() {
        init_vanilla_registry();
        crate::behavior::init_behaviors();
        let fixture = fresh_test_world("village_iron_golem");
        let world = &fixture.world;
        insert_ready_full_chunk(world, ChunkPos::new(0, 0));
        for x in [8.0, 10.0, 12.0] {
            world
                .try_add_entity(villager(world, x))
                .expect("villager must attach");
        }
        for (x, z) in [(8, 10), (10, 10), (12, 10)] {
            world.set_block(
                steel_utils::BlockPos::new(x, 64, z),
                vanilla_blocks::WHITE_BED
                    .default_state()
                    .set_value(&BlockStateProperties::BED_PART, BedPart::Head),
                UpdateFlags::UPDATE_ALL,
            );
        }
        for (x, z) in [(8, 6), (10, 6), (12, 6)] {
            world.set_block(
                steel_utils::BlockPos::new(x, 64, z),
                vanilla_blocks::COMPOSTER.default_state(),
                UpdateFlags::UPDATE_ALL,
            );
        }

        let golem = try_spawn_iron_golem(world, steel_utils::BlockPos::new(10, 64, 8))
            .expect("three villagers + three beds + three composters must spawn a golem");
        let golem = golem
            .downcast_ref::<IronGolemEntity>()
            .expect("iron golem must be concrete");
        assert_eq!(golem.entity_type(), &vanilla_entities::IRON_GOLEM);
        assert!(golem.get_health() > 0.0);
    }

    #[test]
    fn village_below_population_threshold_does_not_spawn_iron_golem() {
        init_vanilla_registry();
        crate::behavior::init_behaviors();
        let fixture = fresh_test_world("village_no_golem");
        let world = &fixture.world;
        insert_ready_full_chunk(world, ChunkPos::new(0, 0));
        for x in [8.0, 10.0] {
            world
                .try_add_entity(villager(world, x))
                .expect("villager must attach");
        }
        for (x, z) in [(8, 10), (10, 10)] {
            world.set_block(
                steel_utils::BlockPos::new(x, 64, z),
                vanilla_blocks::WHITE_BED
                    .default_state()
                    .set_value(&BlockStateProperties::BED_PART, BedPart::Head),
                UpdateFlags::UPDATE_ALL,
            );
        }
        assert!(
            try_spawn_iron_golem(world, steel_utils::BlockPos::new(9, 64, 8)).is_none(),
            "two villagers must not satisfy the {MIN_VILLAGERS}-villager threshold"
        );
    }

    #[test]
    fn village_crop_farm_harvests_and_replants_wheat_with_item_drops() {
        init_vanilla_registry();
        crate::behavior::init_behaviors();
        let fixture = fresh_test_world("village_crop_farm");
        let world = &fixture.world;
        insert_ready_full_chunk(world, ChunkPos::new(0, 0));
        world
            .try_add_entity(villager(world, 10.0))
            .expect("villager must attach");
        world.set_block(
            steel_utils::BlockPos::new(10, 63, 8),
            vanilla_blocks::FARMLAND.default_state(),
            UpdateFlags::UPDATE_ALL,
        );
        world.set_block(
            steel_utils::BlockPos::new(10, 64, 8),
            vanilla_blocks::WHEAT.default_state().set_value(
                &steel_registry::blocks::properties::BlockStateProperties::AGE_7,
                7,
            ),
            UpdateFlags::UPDATE_ALL,
        );
        let (harvested, replanted) =
            super::tick_village_crop_farm(world, steel_utils::BlockPos::new(10, 64, 9));
        assert_eq!((harvested, replanted), (1, 1));
        let state = world.get_block_state(steel_utils::BlockPos::new(10, 64, 8));
        assert_eq!(state.get_block(), &vanilla_blocks::WHEAT);
        assert_eq!(
            state.get_value(&steel_registry::blocks::properties::BlockStateProperties::AGE_7),
            0
        );
        let drops = world.get_entities_in_aabb_matching(
            &steel_utils::WorldAabb::new(9.0, 63.0, 7.0, 12.0, 66.0, 10.0),
            |entity| entity.entity_type() == &vanilla_entities::ITEM,
        );
        assert!(!drops.is_empty(), "wheat harvest must spawn item entities");
    }
}
