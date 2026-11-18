use std::sync::{Arc, LazyLock};
use steel_registry::data_components::vanilla_components::EQUIPPABLE;
use steel_registry::item_stack::ItemStack;
use steel_registry::items::ItemRef;
use steel_registry::data_components::vanilla_components::ENTITY_DATA;
use steel_registry::vanilla_entities;
use steel_registry::vanilla_item_tags::ItemTag;
use steel_registry::{vanilla_blocks, vanilla_items};
use steel_registry::{REGISTRY, RegistryEntry, RegistryExt, TaggedRegistryExt};
use steel_utils::BlockPos;
use steel_utils::BlockStateId;
use steel_utils::Downcast as _;

use crate::entity::entities::{LingeringPotionEntity, SplashPotionEntity, ThrownEggEntity};
use crate::entity::projectile::ThrowableItemProjectile;
use crate::world::World;

pub mod armor;
pub mod arrow;
pub mod block_place;
pub mod bucket;
pub mod consumables;
pub mod container;
pub mod default;
pub mod firework;
pub mod potion;
pub mod projectile;
pub mod spawn_egg;
pub mod tnt;
pub mod tools;

pub use armor::ArmorDispenseBehavior;
pub use arrow::ArrowDispenseBehavior;
pub use block_place::BlockPlaceDispenseBehavior;
pub use bucket::{EmptyBucketDispenseBehavior, FilledBucketDispenseBehavior};
pub use consumables::{
    BoneMealDispenseBehavior, GlowstoneDispenseBehavior, HoneycombDispenseBehavior,
};
pub use container::ShulkerBoxDispenseBehavior;
pub use default::DefaultDispenseBehavior;
pub use firework::FireworkRocketDispenseBehavior;
pub use potion::{GlassBottleDispenseBehavior, PotionDispenseBehavior};
pub use projectile::ProjectileDispenseBehavior;
pub use spawn_egg::SpawnEggDispenseBehavior;
pub use tools::ShearsDispenseBehavior;
pub use tnt::TntDispenseBehavior;
pub use tools::FlintAndSteelDispenseBehavior;

pub trait DispenseItemBehavior: Send + Sync {
    fn dispense(
        &self,
        world: &Arc<World>,
        pos: BlockPos,
        state: BlockStateId,
        item: ItemStack,
    ) -> ItemStack;
}

pub struct DispenseBehaviorRegistry {
    behaviors: Vec<Box<dyn DispenseItemBehavior>>,
}

impl DispenseBehaviorRegistry {
    pub fn new() -> Self {
        let item_count = REGISTRY.items.len();
        let behaviors = (0..item_count)
            .map(|_| Box::new(DefaultDispenseBehavior) as Box<dyn DispenseItemBehavior>)
            .collect();

        Self { behaviors }
    }

    pub fn set_behavior(&mut self, item: ItemRef, behavior: Box<dyn DispenseItemBehavior>) {
        let id = item.id();
        self.behaviors[id] = behavior;
    }

    pub fn get_behavior(&self, item: ItemRef) -> &dyn DispenseItemBehavior {
        let id = item.id();
        self.behaviors[id].as_ref()
    }
}

pub static DISPENSE_BEHAVIORS: LazyLock<DispenseBehaviorRegistry> = LazyLock::new(|| {
    let mut registry = DispenseBehaviorRegistry::new();

    // armor
    for (_, item) in REGISTRY.items.iter() {
        if item.components.has(EQUIPPABLE) {
            registry.set_behavior(item, Box::new(ArmorDispenseBehavior));
        }
    }

    // arrow
    registry.set_behavior(
        &vanilla_items::ARROW,
        Box::new(ArrowDispenseBehavior::new(&vanilla_entities::ARROW)),
    );
    registry.set_behavior(
        &vanilla_items::SPECTRAL_ARROW,
        Box::new(ArrowDispenseBehavior::new(
            &vanilla_entities::SPECTRAL_ARROW,
        )),
    );
    registry.set_behavior(
        &vanilla_items::TIPPED_ARROW,
        Box::new(ArrowDispenseBehavior::new(&vanilla_entities::ARROW)),
    );

    // boat
    // TODO: boats and rafts - need the boat entities.

    // bucket
    registry.set_behavior(
        &vanilla_items::WATER_BUCKET,
        Box::new(FilledBucketDispenseBehavior),
    );
    registry.set_behavior(
        &vanilla_items::LAVA_BUCKET,
        Box::new(FilledBucketDispenseBehavior),
    );
    registry.set_behavior(
        &vanilla_items::POWDER_SNOW_BUCKET,
        Box::new(FilledBucketDispenseBehavior),
    );
    registry.set_behavior(
        &vanilla_items::BUCKET,
        Box::new(EmptyBucketDispenseBehavior),
    );
    // TODO: mob buckets (salmon, cod, pufferfish, tropical fish, axolotl,
    // sulfur cube, tadpole) — vanilla empties them into a mob
    // entity at the target block, but none of those entities/spawn paths
    // are ported yet. Currently fall through to DefaultDispenseBehavior.

    // consumables
    registry.set_behavior(
        &vanilla_items::BONE_MEAL,
        Box::new(BoneMealDispenseBehavior),
    );
    registry.set_behavior(
        &vanilla_items::HONEYCOMB,
        Box::new(HoneycombDispenseBehavior),
    );
    registry.set_behavior(
        &vanilla_items::GLOWSTONE,
        Box::new(GlowstoneDispenseBehavior),
    );
    registry.set_behavior(
        &vanilla_items::CARVED_PUMPKIN,
        Box::new(BlockPlaceDispenseBehavior::new(&vanilla_blocks::CARVED_PUMPKIN)),
    );
    // Vanilla also tries to summon the Wither after placing the third skull.
    // The Wither boss is not implemented yet; the block-placement half is.
    registry.set_behavior(
        &vanilla_items::WITHER_SKELETON_SKULL,
        Box::new(BlockPlaceDispenseBehavior::new(
            &vanilla_blocks::WITHER_SKELETON_SKULL,
        )),
    );

    // potion
    registry.set_behavior(
        &vanilla_items::GLASS_BOTTLE,
        Box::new(GlassBottleDispenseBehavior),
    );
    registry.set_behavior(&vanilla_items::POTION, Box::new(PotionDispenseBehavior));

    // projectile
    for egg in [
        &vanilla_items::EGG,
        &vanilla_items::BLUE_EGG,
        &vanilla_items::BROWN_EGG,
    ] {
        registry.set_behavior(
            egg,
            Box::new(
                ProjectileDispenseBehavior::new(&vanilla_entities::EGG, 1.1, 6.0).with_item_setter(
                    |entity, item| {
                        if let Some(egg) = entity.downcast_ref::<ThrownEggEntity>() {
                            egg.set_item_clamped(item.clone());
                        }
                    },
                ),
            ),
        );
    }
    registry.set_behavior(
        &vanilla_items::SNOWBALL,
        Box::new(ProjectileDispenseBehavior::new(
            &vanilla_entities::SNOWBALL,
            1.1,
            6.0,
        )),
    );
    registry.set_behavior(
        &vanilla_items::ENDER_PEARL,
        Box::new(ProjectileDispenseBehavior::new(
            &vanilla_entities::ENDER_PEARL,
            1.1,
            6.0,
        )),
    );

    registry.set_behavior(
        &vanilla_items::SPLASH_POTION,
        Box::new(
            ProjectileDispenseBehavior::new(&vanilla_entities::SPLASH_POTION, 1.375, 3.0)
                .with_item_setter(|entity, item| {
                    if let Some(potion) = entity.downcast_ref::<SplashPotionEntity>() {
                        potion.set_item_clamped(item.clone());
                    }
                }),
        ),
    );

    registry.set_behavior(
        &vanilla_items::LINGERING_POTION,
        Box::new(
            ProjectileDispenseBehavior::new(&vanilla_entities::LINGERING_POTION, 1.375, 3.0)
                .with_item_setter(|entity, item| {
                    if let Some(potion) = entity.downcast_ref::<LingeringPotionEntity>() {
                        potion.set_item_clamped(item.clone());
                    }
                }),
        ),
    );

    registry.set_behavior(
        &vanilla_items::WIND_CHARGE,
        Box::new(ProjectileDispenseBehavior::new(
            &vanilla_entities::WIND_CHARGE,
            1.1,
            6.0,
        )),
    );
    registry.set_behavior(
        &vanilla_items::FIREWORK_ROCKET,
        Box::new(FireworkRocketDispenseBehavior),
    );
    registry.set_behavior(
        &vanilla_items::EXPERIENCE_BOTTLE,
        Box::new(ProjectileDispenseBehavior::new(
            &vanilla_entities::EXPERIENCE_BOTTLE,
            0.5,
            1.0,
        )),
    );
    registry.set_behavior(
        &vanilla_items::FIRE_CHARGE,
        Box::new(ProjectileDispenseBehavior::new(
            &vanilla_entities::SMALL_FIREBALL,
            1.0,
            0.0,
        )),
    );

    registry.set_behavior(&vanilla_items::TNT, Box::new(TntDispenseBehavior));

    // tools
    registry.set_behavior(
        &vanilla_items::FLINT_AND_STEEL,
        Box::new(FlintAndSteelDispenseBehavior),
    );
    registry.set_behavior(&vanilla_items::SHEARS, Box::new(ShearsDispenseBehavior));
    // TODO: brush - needs the Armadillo entity.

    // container
    for item in REGISTRY.items.iter_tag(&ItemTag::SHULKER_BOXES) {
        let Some(block) = REGISTRY.blocks.by_key(&item.key) else {
            continue;
        };
        registry.set_behavior(item, Box::new(ShulkerBoxDispenseBehavior::new(block)));
    }
    // TODO: chest - equips tamed donkeys/mules/llamas; needs those entities.

    // minecart
    // TODO: minecart and chest minecart - entities exist; port
    // MinecartDispenseItemBehavior. Furnace/hopper/tnt/command-block
    // minecarts also need their entities.

    // spawn egg: data-driven by ENTITY_DATA. Match the vanilla item path
    // instead of component presence so ordinary block items cannot be caught.
    for (_, item) in REGISTRY.items.iter() {
        if item.key.path.ends_with("spawn_egg") && item.components.has(ENTITY_DATA) {
            registry.set_behavior(item, Box::new(SpawnEggDispenseBehavior));
        }
    }
    // TODO: armor stand - needs the ArmorStand entity.

    registry
});
