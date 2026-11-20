//! Minimal village POI manager: beds + workstations + villagers drive iron golem spawning.
//!
//! This is deliberately data-driven rather than tied to a structure: it scans loaded
//! blocks around a village centre, counts valid POIs, and spawns an iron golem when the
//! vanilla population/bed/workstation thresholds are met.

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::LazyLock;
use std::sync::atomic::{AtomicUsize, Ordering};

use glam::DVec3;
use steel_registry::blocks::block_state_ext::BlockStateExt as _;
use steel_registry::blocks::properties::{BlockStateProperties, IntProperty};
use steel_registry::blocks::{Block, BlockRef};
use steel_registry::fluid::FluidStateExt as _;
use steel_registry::item_stack::ItemStack;
use steel_registry::items::{Item, ItemRef};
use steel_registry::{vanilla_blocks, vanilla_entities, vanilla_items};
use steel_utils::locks::SyncMutex;
use steel_utils::random::Random as _;
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

/// Checks whether an iron golem could stand at `feet`: a solid block below,
/// two free body blocks, and no lava or water at the feet.
#[must_use]
fn golem_spawn_position_valid(world: &World, feet: BlockPos) -> bool {
    if !world.is_in_valid_bounds(feet) {
        return false;
    }
    let below = world.get_block_state(feet.below());
    if !below.is_solid() {
        return false;
    }
    let feet_state = world.get_block_state(feet);
    let head_state = world.get_block_state(feet.above());
    if !feet_state.is_air() || !head_state.is_air() {
        return false;
    }
    let feet_fluid = feet_state.get_fluid_state();
    if feet_fluid.is_lava() || feet_fluid.is_water() {
        return false;
    }
    true
}

/// Searches the blocks immediately around `center` for a valid golem spawn
/// position, preferring the centre itself and its nearest neighbours.
#[must_use]
fn find_golem_spawn(world: &World, center: BlockPos) -> Option<BlockPos> {
    let horizontal = [0, -1, 1, -2, 2];
    for dx in horizontal {
        for dz in horizontal {
            for dy in [-1, 0, 1] {
                let feet = center.offset(dx, dy, dz);
                if golem_spawn_position_valid(world, feet) {
                    return Some(feet);
                }
            }
        }
    }
    None
}

/// Tries to spawn an iron golem for a village at `center`.
///
/// Returns the spawned golem when the population/bed/workstation thresholds
/// pass and a valid standing position could be found.
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

    let Some(feet) = find_golem_spawn(world, center) else {
        log::debug!("iron golem: no valid standing position near {center:?}");
        return None;
    };
    let spawn = DVec3::new(
        f64::from(feet.x()) + 0.5,
        f64::from(feet.y()),
        f64::from(feet.z()) + 0.5,
    );
    let golem: SharedEntity = Arc::new(IronGolemEntity::new(
        &vanilla_entities::IRON_GOLEM,
        next_entity_id(),
        spawn,
        Arc::downgrade(world),
    ));
    if let Err(error) = world.try_add_entity(Arc::clone(&golem)) {
        log::debug!("failed to add iron golem at {feet:?}: {error}");
        return None;
    }
    Some(golem)
}

/// Describes one mature crop the villagers can harvest and replant.
struct CropDefinition {
    crop_block: BlockRef,
    age_property: &'static IntProperty,
    mature_age: u8,
    product: ItemRef,
    seed: Option<ItemRef>,
}

/// Harvestable village crops: wheat, carrots, potatoes and beetroots.
static CROP_DEFINITIONS: LazyLock<Vec<CropDefinition>> = LazyLock::new(|| {
    vec![
        CropDefinition {
            crop_block: &vanilla_blocks::WHEAT,
            age_property: &BlockStateProperties::AGE_7,
            mature_age: 7,
            product: &vanilla_items::WHEAT,
            seed: Some(&vanilla_items::WHEAT_SEEDS),
        },
        CropDefinition {
            crop_block: &vanilla_blocks::CARROTS,
            age_property: &BlockStateProperties::AGE_7,
            mature_age: 7,
            product: &vanilla_items::CARROT,
            seed: None,
        },
        CropDefinition {
            crop_block: &vanilla_blocks::POTATOES,
            age_property: &BlockStateProperties::AGE_7,
            mature_age: 7,
            product: &vanilla_items::POTATO,
            seed: None,
        },
        CropDefinition {
            crop_block: &vanilla_blocks::BEETROOTS,
            age_property: &BlockStateProperties::AGE_3,
            mature_age: 3,
            product: &vanilla_items::BEETROOT,
            seed: Some(&vanilla_items::BEETROOT_SEEDS),
        },
    ]
});

fn crop_definition(block: &Block) -> Option<&'static CropDefinition> {
    CROP_DEFINITIONS.iter().find(|crop| crop.crop_block == block)
}

/// Harvests mature crops within `radius` of `center` and replants them.
///
/// Returns `(harvested, replanted)` counts. Drops are real item entities so
/// hoppers can collect them; seed drops use the world's seeded random source
/// and roll 0..=2.
pub fn harvest_mature_crops_near(
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
                let Some(crop) = crop_definition(state.get_block()) else {
                    continue;
                };
                if state.get_value(crop.age_property) < crop.mature_age {
                    continue;
                }
                world.set_block(
                    pos,
                    vanilla_blocks::AIR.default_state(),
                    steel_utils::types::UpdateFlags::UPDATE_ALL,
                );
                let drop_pos = glam::DVec3::new(
                    f64::from(pos.x()) + 0.5,
                    f64::from(pos.y()) + 0.5,
                    f64::from(pos.z()) + 0.5,
                );
                world.spawn_item(drop_pos, ItemStack::new(crop.product));
                if let Some(seed) = crop.seed {
                    let seed_count = world
                        .loot_random
                        .lock()
                        .next_i32_between(0, 2);
                    if seed_count > 0 {
                        world.spawn_item(drop_pos, ItemStack::with_count(seed, seed_count));
                    }
                }
                world.set_block(
                    pos,
                    crop.crop_block.default_state(),
                    steel_utils::types::UpdateFlags::UPDATE_ALL,
                );
                harvested += 1;
                replanted += 1;
            }
        }
    }
    (harvested, replanted)
}

/// Harvests mature crops within four blocks of each village and replants it.
///
/// Returns `(harvested, replanted)` counts. This is the village crop-farm loop:
/// villagers are the workforce, farmland/crops are the POIs, and drops are real
/// item entities so hoppers can collect them.
pub fn tick_village_crop_farm(world: &Arc<World>, center: BlockPos) -> (usize, usize) {
    let villagers = nearby_villagers(world, center, f64::from(VILLAGE_POI_RADIUS));
    let mut harvested = 0;
    let mut replanted = 0;
    for villager in villagers {
        let (h, r) = harvest_mature_crops_near(world, villager.block_position(), 4);
        harvested += h;
        replanted += r;
    }
    (harvested, replanted)
}

/// Village-level aggregation window: the crop-farm scan and iron-golem attempt
/// run at most once per village chunk per this many game ticks, instead of once
/// per villager. Thresholds themselves are unchanged.
const VILLAGE_AGGREGATION_INTERVAL: i64 = 200;

static LAST_VILLAGE_AGGREGATION: LazyLock<SyncMutex<HashMap<(usize, i32, i32), i64>>> =
    LazyLock::new(|| SyncMutex::new(HashMap::new()));
static VILLAGE_ATTEMPT_COUNT: AtomicUsize = AtomicUsize::new(0);

/// Number of aggregated village attempts executed since process start.
/// Exposed for tests to assert multi-villager dedup.
#[must_use]
pub fn village_attempt_count() -> usize {
    VILLAGE_ATTEMPT_COUNT.load(Ordering::Relaxed)
}

/// Runs the village crop-farm scan and iron-golem attempt for the chunk owning
/// `center`, unless the aggregated run already happened within the interval.
///
/// Returns `(harvested, replanted, golem_attempted)`.
pub fn tick_village_if_due(world: &Arc<World>, center: BlockPos) -> (usize, usize, bool) {
    // The owning world is part of the key: aggregation state must not leak
    // across worlds (or across test worlds sharing chunk coordinates).
    let world_key = Arc::as_ptr(&world.game_time) as usize;
    let key = (world_key, center.x() >> 4, center.z() >> 4);
    let now = world.game_time();
    {
        let mut last = LAST_VILLAGE_AGGREGATION.lock();
        let due = last
            .get(&key)
            .map_or(true, |&tick| now - tick >= VILLAGE_AGGREGATION_INTERVAL);
        if !due {
            return (0, 0, false);
        }
        last.insert(key, now);
    }
    VILLAGE_ATTEMPT_COUNT.fetch_add(1, Ordering::Relaxed);

    let (harvested, replanted) = tick_village_crop_farm(world, center);
    let golem = try_spawn_iron_golem(world, center);
    (harvested, replanted, golem.is_some())
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
    use crate::entity::damage::DamageSource;
    use crate::entity::entities::{IronGolemEntity, ItemEntity, VillagerEntity};
    use crate::entity::{Entity, LivingEntity, SharedEntity, next_entity_id};
    use crate::inventory::lock::{ContainerLockGuard, ContainerRef};
    use crate::test_support::{fresh_test_world, insert_ready_full_chunk};
    use steel_registry::{vanilla_damage_types, vanilla_items};

    fn villager(world: &Arc<crate::world::World>, x: f64) -> SharedEntity {
        Arc::new(VillagerEntity::new(
            &vanilla_entities::VILLAGER,
            next_entity_id(),
            DVec3::new(x, 64.0, 8.0),
            Arc::downgrade(world),
        ))
    }

    fn place_stone_floor(
        world: &Arc<crate::world::World>,
        y: i32,
        x0: i32,
        x1: i32,
        z0: i32,
        z1: i32,
    ) {
        for x in x0..=x1 {
            for z in z0..=z1 {
                world.set_block(
                    steel_utils::BlockPos::new(x, y, z),
                    vanilla_blocks::STONE.default_state(),
                    UpdateFlags::UPDATE_ALL,
                );
            }
        }
    }

    #[test]
    fn village_with_three_villagers_beds_and_composters_spawns_iron_golem() {
        init_vanilla_registry();
        crate::behavior::init_behaviors();
        let fixture = fresh_test_world("village_iron_golem");
        let world = &fixture.world;
        insert_ready_full_chunk(world, ChunkPos::new(0, 0));
        place_stone_floor(world, 63, 8, 12, 6, 10);
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
    fn villager_ai_harvests_crop_farm_and_hopper_collects_wheat() {
        init_vanilla_registry();
        crate::behavior::init_behaviors();
        crate::block_entity::init_block_entities();
        let fixture = fresh_test_world("village_crop_auto");
        let world = &fixture.world;
        insert_ready_full_chunk(world, ChunkPos::new(0, 0));
        place_stone_floor(world, 63, 7, 10, 7, 9);

        let villager = villager(world, 8.0);
        world
            .try_add_entity(Arc::clone(&villager))
            .expect("villager must attach");
        let concrete = villager
            .downcast_ref::<VillagerEntity>()
            .expect("villager concrete");

        let farmland = steel_utils::BlockPos::new(8, 63, 8);
        let wheat = steel_utils::BlockPos::new(8, 64, 8);
        world.set_block(farmland, vanilla_blocks::FARMLAND.default_state(), UpdateFlags::UPDATE_ALL);
        world.set_block(
            wheat,
            vanilla_blocks::WHEAT.default_state().set_value(
                &BlockStateProperties::AGE_7,
                7,
            ),
            UpdateFlags::UPDATE_ALL,
        );
        let hopper = steel_utils::BlockPos::new(9, 63, 8);
        let chest = steel_utils::BlockPos::new(9, 62, 8);
        world.set_block(hopper, vanilla_blocks::HOPPER.default_state(), UpdateFlags::UPDATE_ALL);
        world.set_block(chest, vanilla_blocks::CHEST.default_state(), UpdateFlags::UPDATE_ALL);

        // The villager's production AI runs the village crop loop every 200gt.
        for _ in 0..200 {
            concrete.ai_step(&villager);
        }
        assert_eq!(
            world.get_block_state(wheat).get_block(),
            &vanilla_blocks::WHEAT,
            "villager must replant the harvested wheat"
        );

        let drops = world.get_entities_in_aabb_matching(
            &steel_utils::WorldAabb::new(6.0, 63.0, 6.0, 11.0, 67.0, 10.0),
            |entity| entity.entity_type() == &vanilla_entities::ITEM,
        );
        for entity in &drops {
            if entity
                .downcast_ref::<ItemEntity>()
                .is_some_and(|item| item.get_item().is(&vanilla_items::WHEAT))
            {
                let _ = entity.teleport_to(glam::DVec3::new(9.5, 64.2, 8.5));
            }
        }
        for _ in 0..40 {
            if let Some(block_entity) = world.get_block_entity(hopper) {
                block_entity.tick(world);
            }
        }
        let chest_ref = ContainerRef::from_block_entity(
            world.get_block_entity(chest).expect("chest"),
        )
        .expect("chest container");
        let guard = ContainerLockGuard::lock_all(&[&chest_ref]).expect("lock chest");
        let inventory = guard.get(chest_ref.container_id()).expect("chest inventory");
        assert!(
            (0..27).any(|slot| inventory.get_item(slot).is(&vanilla_items::WHEAT)),
            "hopper must collect the villager-harvested wheat"
        );
    }

    #[test]
    fn iron_golem_farm_drops_iron_ingots_into_hopper() {
        init_vanilla_registry();
        crate::behavior::init_behaviors();
        crate::block_entity::init_block_entities();
        let fixture = fresh_test_world("village_iron_farm");
        let world = &fixture.world;
        insert_ready_full_chunk(world, ChunkPos::new(0, 0));
        place_stone_floor(world, 63, 8, 12, 6, 10);
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
            .expect("village thresholds must spawn a golem");
        let concrete = golem.downcast_ref::<IronGolemEntity>().expect("golem concrete");

        let hopper = steel_utils::BlockPos::new(10, 64, 8);
        let chest = steel_utils::BlockPos::new(10, 63, 8);
        world.set_block(hopper, vanilla_blocks::HOPPER.default_state(), UpdateFlags::UPDATE_ALL);
        world.set_block(chest, vanilla_blocks::CHEST.default_state(), UpdateFlags::UPDATE_ALL);

        let lava = DamageSource::environment(&vanilla_damage_types::LAVA);
        assert!(concrete.hurt(world, &lava, 200.0), "lava must kill the golem");
        for _ in 0..20 {
            Arc::clone(&golem).tick();
        }
        let drops = world.get_entities_in_aabb_matching(
            &steel_utils::WorldAabb::new(7.0, 63.0, 7.0, 13.0, 67.0, 9.0),
            |entity| entity.entity_type() == &vanilla_entities::ITEM,
        );
        assert!(
            drops.iter().any(|entity| entity
                .downcast_ref::<ItemEntity>()
                .is_some_and(|item| item.get_item().is(&vanilla_items::IRON_INGOT))),
            "iron golem must drop iron ingots"
        );
        let drops = world.get_entities_in_aabb_matching(
            &steel_utils::WorldAabb::new(6.0, 63.0, 6.0, 11.0, 67.0, 10.0),
            |entity| entity.entity_type() == &vanilla_entities::ITEM,
        );
        for entity in &drops {
            if entity
                .downcast_ref::<ItemEntity>()
                .is_some_and(|item| item.get_item().is(&vanilla_items::WHEAT))
            {
                let _ = entity.teleport_to(glam::DVec3::new(9.5, 64.2, 8.5));
            }
        }
        for _ in 0..40 {
            if let Some(block_entity) = world.get_block_entity(hopper) {
                block_entity.tick(world);
            }
        }
        let chest_ref = ContainerRef::from_block_entity(
            world.get_block_entity(chest).expect("chest"),
        )
        .expect("chest container");
        let guard = ContainerLockGuard::lock_all(&[&chest_ref]).expect("lock chest");
        let inventory = guard.get(chest_ref.container_id()).expect("chest inventory");
        assert!(
            (0..27).any(|slot| inventory.get_item(slot).is(&vanilla_items::IRON_INGOT)),
            "iron golem farm hopper must collect iron ingots"
        );
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

    #[test]
    fn golem_spawn_position_valid_accepts_clear_standable_position() {
        init_vanilla_registry();
        let fixture = fresh_test_world("golem_spawn_valid");
        let world = &fixture.world;
        insert_ready_full_chunk(world, ChunkPos::new(0, 0));

        let below = steel_utils::BlockPos::new(10, 63, 8);
        let feet = steel_utils::BlockPos::new(10, 64, 8);
        world.set_block(below, vanilla_blocks::STONE.default_state(), UpdateFlags::UPDATE_ALL);

        assert!(super::golem_spawn_position_valid(world, feet));
    }

    #[test]
    fn golem_spawn_position_valid_rejects_blocked_body_or_missing_floor() {
        init_vanilla_registry();
        let fixture = fresh_test_world("golem_spawn_invalid_body");
        let world = &fixture.world;
        insert_ready_full_chunk(world, ChunkPos::new(0, 0));

        let feet = steel_utils::BlockPos::new(10, 64, 8);
        // No solid floor below.
        assert!(!super::golem_spawn_position_valid(world, feet));

        world.set_block(
            feet,
            vanilla_blocks::STONE.default_state(),
            UpdateFlags::UPDATE_ALL,
        );
        // Block occupying the feet position.
        assert!(!super::golem_spawn_position_valid(world, feet));

        let head = steel_utils::BlockPos::new(10, 65, 8);
        world.set_block(
            head,
            vanilla_blocks::STONE.default_state(),
            UpdateFlags::UPDATE_ALL,
        );
        // Block occupying the head position.
        assert!(!super::golem_spawn_position_valid(world, feet));
    }

    #[test]
    fn golem_spawn_position_valid_rejects_water_and_lava_feet() {
        init_vanilla_registry();
        let fixture = fresh_test_world("golem_spawn_fluid");
        let world = &fixture.world;
        insert_ready_full_chunk(world, ChunkPos::new(0, 0));

        let below = steel_utils::BlockPos::new(10, 63, 8);
        let feet = steel_utils::BlockPos::new(10, 64, 8);
        world.set_block(below, vanilla_blocks::STONE.default_state(), UpdateFlags::UPDATE_ALL);

        world.set_block(feet, vanilla_blocks::WATER.default_state(), UpdateFlags::UPDATE_ALL);
        assert!(!super::golem_spawn_position_valid(world, feet));

        world.set_block(feet, vanilla_blocks::LAVA.default_state(), UpdateFlags::UPDATE_ALL);
        assert!(!super::golem_spawn_position_valid(world, feet));
    }

    #[test]
    fn village_aggregation_runs_once_per_chunk_despite_many_villagers() {
        init_vanilla_registry();
        crate::behavior::init_behaviors();
        let fixture = fresh_test_world("village_dedup");
        let world = &fixture.world;
        insert_ready_full_chunk(world, ChunkPos::new(0, 0));

        let before = super::village_attempt_count();
        // Six villagers would each trigger a village scan before aggregation;
        // the chunk-level gate must collapse them into a single attempt.
        for _ in 0..6 {
            super::tick_village_if_due(world, steel_utils::BlockPos::new(10, 64, 8));
        }
        assert_eq!(
            super::village_attempt_count() - before,
            1,
            "village-level aggregation must run at most once per chunk window"
        );
    }
}
