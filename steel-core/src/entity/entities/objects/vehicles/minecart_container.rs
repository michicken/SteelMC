//! Inventory and loot state for a container minecart.

use std::sync::Arc;

use glam::DVec3;

use simdnbt::ToNbtTag as _;
use simdnbt::borrow::NbtCompound as BorrowedNbtCompound;
use simdnbt::owned::{NbtCompound, NbtList, NbtTag};
use steel_registry::RegistryExt as _;
use steel_registry::data_components::vanilla_components::SeededContainerLoot;
use steel_registry::item_stack::ItemStack;
use steel_utils::locks::SyncMutex;
use steel_utils::{ChunkPos, DowncastType, DowncastTypeKey, Identifier};

use crate::behavior::InteractionResult;
use crate::entity::entities::objects::vehicles::MinecartEntity;
use crate::entity::{
    Entity, LivingEntity as _, SharedEntity, WeakEntity, entity_loot_ref, living_loot_equipment,
};
use crate::inventory::container::{
    Container, calculate_redstone_signal_from_containers, load_all_items,
};
use crate::inventory::lock::{ContainerId, ContainerRef};
use crate::inventory::menu::kinds::generic_9x_menu_with_kind;
use crate::inventory::menu::{MenuBehavior, MenuKind};
use crate::loot::{GeneratedLoot, LOOT_TABLES, LootContext, LootError, LootResult};
use crate::player::Player;

const CHEST_SLOTS: usize = 27;

pub(super) struct MinecartContainer {
    inventory: Arc<SyncMutex<MinecartInventory>>,
    removal: SyncMutex<()>,
}

struct MinecartInventory {
    items: Vec<ItemStack>,
    loot: Option<SeededContainerLoot>,
    generation: u64,
    unpacking: bool,
}

// SAFETY: The key uniquely identifies Steel's minecart inventory.
unsafe impl DowncastType for MinecartInventory {
    const TYPE_KEY: DowncastTypeKey = DowncastTypeKey::new("steel:container/minecart");
}

impl Container for MinecartInventory {
    fn items(&self) -> &[ItemStack] {
        &self.items
    }
    fn items_mut(&mut self) -> &mut [ItemStack] {
        &mut self.items
    }
    fn set_item(&mut self, slot: usize, mut item: ItemStack) {
        if slot >= self.items.len() {
            return;
        }
        let limit = self.get_max_stack_size_for_item(&item);
        if !item.is_empty() && item.count() > limit {
            item.set_count(limit);
        }
        self.items[slot] = item;
    }

    fn set_changed(&mut self) {}
}

impl MinecartContainer {
    pub(super) fn new() -> Self {
        Self {
            inventory: Arc::new(SyncMutex::new(MinecartInventory {
                items: vec![ItemStack::empty(); CHEST_SLOTS],
                loot: None,
                generation: 0,
                unpacking: false,
            })),
            removal: SyncMutex::new(()),
        }
    }

    pub(super) fn interact(&self, cart: &MinecartEntity, player: &Player) -> InteractionResult {
        if player.is_spectator() && self.has_loot() {
            return InteractionResult::Success;
        }
        let Some(world) = cart.level() else {
            return InteractionResult::Pass;
        };
        let Some(entity) = world.get_entity_by_id(cart.id()) else {
            return InteractionResult::Pass;
        };
        if let Err(error) = self.unpack_for_player(&entity, player) {
            log::error!("Could not unpack minecart loot: {error}");
            return InteractionResult::Success;
        }
        let container = self.container_ref(&entity);
        let weak = Arc::downgrade(&entity);
        let inventory = Arc::clone(&player.inventory);
        let title = cart.name();
        player.open_menu(title, move |context| {
            let kind = MinecartMenu {
                entity: weak,
                container: container.container_id(),
            };
            generic_9x_menu_with_kind(inventory, context.container_id, container, 3, kind)
        });
        cart.game_event_with_source_entity(
            &steel_registry::vanilla_game_events::CONTAINER_OPEN,
            Some(player),
        );
        InteractionResult::Success
    }

    pub(super) fn slowdown(&self, cart: &MinecartEntity, motion: DVec3) -> DVec3 {
        let state = self.inventory.lock();
        let mut factor = 0.98_f32;
        if state.loot.is_none() {
            let fullness = calculate_redstone_signal_from_containers(std::iter::once(
                &*state as &dyn Container,
            ));
            factor += (15 - fullness) as f32 * 0.001;
        }
        if cart.is_in_water() {
            factor *= 0.95;
        }
        motion * DVec3::new(f64::from(factor), 0.0, f64::from(factor))
    }

    pub(super) fn set_loot(&self, table: Identifier, seed: i64) {
        let _removal = self.removal.lock();
        let mut inventory = self.inventory.lock();
        inventory.loot = Some(SeededContainerLoot::new(table, seed));
        inventory.generation = inventory.generation.wrapping_add(1);
        inventory.unpacking = false;
    }

    pub(super) fn has_loot(&self) -> bool {
        self.inventory.lock().loot.is_some()
    }

    pub(super) fn container_ref(&self, entity: &SharedEntity) -> ContainerRef {
        let entity: WeakEntity = Arc::downgrade(entity);
        let changed_entity = entity.clone();
        let inventory = Arc::downgrade(&self.inventory);
        ContainerRef::from(&self.inventory)
            .with_access_callback(Arc::new(move || {
                let (Some(entity), Some(inventory)) = (entity.upgrade(), inventory.upgrade())
                else {
                    return Err(LootError::ContainerChanged);
                };
                if entity.is_removed() {
                    return Err(LootError::ContainerChanged);
                }
                Self::unpack(&inventory, &entity, None)
            }))
            .with_changed_callback(Arc::new(move || {
                if let Some(entity) = changed_entity.upgrade()
                    && let Some(world) = entity.level()
                {
                    world.mark_chunk_dirty(ChunkPos::from_entity_pos(entity.position()));
                }
            }))
    }

    pub(super) fn unpack_for_player(
        &self,
        entity: &SharedEntity,
        player: &Player,
    ) -> LootResult<()> {
        Self::unpack(&self.inventory, entity, Some(player))
    }

    fn unpack(
        inventory: &Arc<SyncMutex<MinecartInventory>>,
        entity: &SharedEntity,
        player: Option<&Player>,
    ) -> LootResult<()> {
        let Some(world) = entity.level() else {
            return Ok(());
        };
        let (loot, generation) = {
            let mut state = inventory.lock();
            if state.unpacking {
                return Err(LootError::ContainerChanged);
            }
            let Some(loot) = state.loot.clone() else {
                return Ok(());
            };
            state.unpacking = true;
            state.generation = state.generation.wrapping_add(1);
            (loot, state.generation)
        };
        let table = LOOT_TABLES.by_key(loot.loot_table());
        let equipment = player.map(living_loot_equipment);
        let result = world.with_loot_random(table, loot.seed(), |random| {
            let mut context = LootContext::new(random)
                .with_world(&world)
                .with_origin(entity.position());
            if let Some(player) = player {
                context = context
                    .with_luck(player.get_luck())
                    .with_this_entity(entity_loot_ref(player, equipment.as_ref()));
            }
            let generated = match table {
                Some(table) => table.generate_for_container(&mut context)?,
                None => GeneratedLoot::new(Vec::new(), context.rng),
            };
            let mut state = inventory.lock();
            if state.generation != generation {
                return Err(LootError::ContainerChanged);
            }
            state.loot = None;
            generated.fill(&mut *state);
            Ok(())
        });
        let mut state = inventory.lock();
        if state.generation == generation {
            state.unpacking = false;
        }
        drop(state);
        if result.is_ok() {
            world.mark_chunk_dirty(ChunkPos::from_entity_pos(entity.position()));
        }
        result
    }

    pub(super) fn save(&self, nbt: &mut NbtCompound) {
        let _removal = self.removal.lock();
        let state = self.inventory.lock();
        if let Some(loot) = &state.loot {
            nbt.insert("LootTable", loot.loot_table().to_string());
            if loot.seed() != 0 {
                nbt.insert("LootTableSeed", loot.seed());
            }
            return;
        }
        let mut items = Vec::new();
        for (slot, item) in state.items.iter().enumerate() {
            if !item.is_empty()
                && let NbtTag::Compound(mut tag) = item.clone().to_nbt_tag()
            {
                tag.insert("Slot", slot as i8);
                items.push(tag);
            }
        }
        nbt.insert("Items", NbtList::Compound(items));
    }

    pub(super) fn load(&self, nbt: BorrowedNbtCompound<'_, '_>) {
        let _removal = self.removal.lock();
        let mut state = self.inventory.lock();
        state.items.fill(ItemStack::empty());
        state.loot = nbt
            .string("LootTable")
            .and_then(|key| key.to_string().parse().ok())
            .map(|key| SeededContainerLoot::new(key, nbt.long("LootTableSeed").unwrap_or(0)));
        state.generation = state.generation.wrapping_add(1);
        state.unpacking = false;
        if state.loot.is_none() {
            load_all_items(nbt, &mut state.items);
        }
    }

    pub(super) fn remove(&self, cart: &MinecartEntity, reason: crate::entity::RemovalReason) {
        let _guard = self.removal.lock();
        if cart.is_removed() {
            return;
        }
        if reason.should_destroy()
            && let Some(world) = cart.level()
            && world.get_game_rule(&steel_registry::vanilla_game_rules::ENTITY_DROPS)
            && let Some(entity) = world.get_entity_by_id(cart.id())
        {
            if let Err(error) = self.drop_contents(&entity) {
                log::error!("Could not drop minecart contents: {error}");
                return;
            }
        }
        cart.base().set_removed(reason);
    }

    fn drop_contents(&self, entity: &SharedEntity) -> LootResult<()> {
        Self::unpack(&self.inventory, entity, None)?;
        let items = {
            let mut state = self.inventory.lock();
            if state.unpacking {
                return Err(LootError::ContainerChanged);
            }
            state.unpacking = true;
            std::mem::replace(&mut state.items, vec![ItemStack::empty(); CHEST_SLOTS])
        };
        let mut failed = Vec::new();
        for (slot, item) in items.into_iter().enumerate() {
            if !item.is_empty() && entity.spawn_at_location(item.clone(), 0.0).is_none() {
                failed.push((slot, item));
            }
        }
        let mut state = self.inventory.lock();
        for (slot, item) in failed {
            state.items[slot] = item;
        }
        state.unpacking = false;
        let failed = state.items.iter().any(|item| !item.is_empty());
        drop(state);
        if failed {
            if let Some(world) = entity.level() {
                world.mark_chunk_dirty(ChunkPos::from_entity_pos(entity.position()));
            }
            Err(LootError::ContainerChanged)
        } else {
            Ok(())
        }
    }
}

struct MinecartMenu {
    entity: WeakEntity,
    container: ContainerId,
}

// SAFETY: This key uniquely identifies Steel's minecart menu.
unsafe impl DowncastType for MinecartMenu {
    const TYPE_KEY: DowncastTypeKey = DowncastTypeKey::new("steel:menu/minecart_chest");
}

impl MenuKind for MinecartMenu {
    fn opened_containers(&self) -> smallvec::SmallVec<[ContainerId; 2]> {
        smallvec::smallvec![self.container]
    }

    fn still_valid(&self, _behavior: &MenuBehavior, player: &Player) -> bool {
        self.entity.upgrade().is_some_and(|entity| {
            !entity.is_removed()
                && player.is_within_entity_interaction_range(entity.bounding_box(), 4.0)
        })
    }

    fn removed(&mut self, _behavior: &mut MenuBehavior, player: &Player) {
        if let Some(entity) = self.entity.upgrade() {
            entity.game_event_with_source_entity(
                &steel_registry::vanilla_game_events::CONTAINER_CLOSE,
                Some(player),
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::entity::entities::objects::vehicles::test_support::test_world;
    use crate::entity::{ENTITIES, next_entity_id};
    use crate::inventory::lock::ContainerLockGuard;
    use crate::player::ResetReason;
    use crate::test_support::TestPlayerBuilder;
    use glam::DVec3;
    use simdnbt::borrow::read_compound;
    use std::io::Cursor;
    use steel_registry::{vanilla_entities, vanilla_items};
    use steel_utils::{ChunkPos, Downcast as _, types::InteractionHand};

    #[test]
    fn chest_minecart_factory_opens_and_preserves_inventory() {
        let fixture = test_world("minecart_chest_inventory", ChunkPos::new(0, 0));
        let world = &fixture.world;
        let cart = ENTITIES
            .create(
                &vanilla_entities::CHEST_MINECART,
                DVec3::new(8.5, 64.0, 8.5),
                Arc::downgrade(world),
            )
            .expect("factory creates chest minecart");
        world
            .try_add_entity(Arc::clone(&cart))
            .expect("loaded chunk");
        let chunk_pos = ChunkPos::new(0, 0);
        world
            .chunk_map
            .with_chunk_at_status(
                chunk_pos,
                crate::chunk::status::ChunkStatus::Empty,
                |chunk| chunk.clear_dirty(),
            )
            .expect("cart chunk");
        let container = cart.container_ref().expect("chest inventory");
        let mut guard = ContainerLockGuard::lock_all(&[&container]).expect("inventory");
        assert!(guard.set_item(
            container.container_id(),
            0,
            ItemStack::with_count(&vanilla_items::DIAMOND, 3)
        ));
        drop(guard);
        assert!(
            world
                .chunk_map
                .with_chunk_at_status(
                    chunk_pos,
                    crate::chunk::status::ChunkStatus::Empty,
                    |chunk| chunk.is_dirty(),
                )
                .expect("cart chunk")
        );

        let mut saved = NbtCompound::new();
        cart.save_additional(&mut saved);
        let mut bytes = Vec::new();
        saved.write(&mut bytes);
        let borrowed = read_compound(&mut Cursor::new(bytes.as_slice())).expect("NBT");
        let restored = MinecartEntity::new(
            &vanilla_entities::CHEST_MINECART,
            next_entity_id(),
            DVec3::ZERO,
            Arc::downgrade(world),
        );
        restored.load_additional((&borrowed).into());
        let mut resaved = NbtCompound::new();
        restored.save_additional(&mut resaved);
        assert_eq!(saved, resaved);

        let player =
            TestPlayerBuilder::new(Arc::clone(world), "CartPlayer", next_entity_id()).build();
        player.set_client_loaded(true);
        assert!(world.add_player(Arc::clone(&player), ResetReason::InitialJoin));
        assert_eq!(
            cart.interact(&player, InteractionHand::MainHand, DVec3::ZERO),
            InteractionResult::Success
        );
        assert!(player.has_container_open());
    }

    #[test]
    fn destroyed_chest_minecart_drops_its_contents_once() {
        let fixture = test_world("minecart_chest_drops", ChunkPos::new(0, 0));
        let world = &fixture.world;
        let position = DVec3::new(8.5, 64.0, 8.5);
        let cart: SharedEntity = Arc::new(MinecartEntity::new(
            &vanilla_entities::CHEST_MINECART,
            next_entity_id(),
            position,
            Arc::downgrade(world),
        ));
        world
            .try_add_entity(Arc::clone(&cart))
            .expect("loaded cart");
        let container = cart.container_ref().expect("cart inventory");
        let mut guard = ContainerLockGuard::lock_all(&[&container]).expect("inventory");
        assert!(guard.set_item(
            container.container_id(),
            0,
            ItemStack::with_count(&vanilla_items::DIAMOND, 3),
        ));
        drop(guard);
        cart.set_removed(crate::entity::RemovalReason::Killed);
        cart.set_removed(crate::entity::RemovalReason::Killed);
        let drops = crate::test_support::dropped_items(world, position);
        let total: i32 = drops
            .iter()
            .filter_map(|entity| {
                entity
                    .downcast_ref::<crate::entity::entities::ItemEntity>()
                    .map(|item| item.get_item().count())
            })
            .sum();
        assert_eq!(total, 3);
    }

    #[test]
    fn failed_inventory_preparation_does_not_destroy_or_drop_the_cart() {
        use crate::entity::DamageSource;
        use crate::entity::entities::objects::vehicles::VehicleEntity;
        use steel_registry::vanilla_damage_types;

        let fixture = test_world("minecart_chest_busy", ChunkPos::new(0, 0));
        let world = &fixture.world;
        let position = DVec3::new(8.5, 64.0, 8.5);
        let cart = Arc::new(MinecartEntity::new(
            &vanilla_entities::CHEST_MINECART,
            next_entity_id(),
            position,
            Arc::downgrade(world),
        ));
        world
            .try_add_entity(Arc::clone(&cart) as SharedEntity)
            .expect("loaded cart");
        let chest = cart.chest.as_ref().expect("chest");
        chest.inventory.lock().unpacking = true;
        cart.vehicle_destroy(
            world,
            &DamageSource::environment(&vanilla_damage_types::GENERIC),
        );
        assert!(!cart.is_removed());
        assert!(crate::test_support::dropped_items(world, position).is_empty());
        chest.inventory.lock().unpacking = false;
    }

    #[test]
    fn chest_minecart_generated_loot_survives_save_until_opened() {
        let fixture = test_world("minecart_chest_loot", ChunkPos::new(0, 0));
        let world = &fixture.world;
        let cart = Arc::new(MinecartEntity::new(
            &vanilla_entities::CHEST_MINECART,
            next_entity_id(),
            DVec3::new(8.5, 64.0, 8.5),
            Arc::downgrade(world),
        ));
        cart.set_loot_table(Identifier::vanilla_static("chests/spawn_bonus_chest"), 42);
        let entity: SharedEntity = cart;
        world
            .try_add_entity(Arc::clone(&entity))
            .expect("loaded chunk");
        let mut saved = NbtCompound::new();
        entity.save_additional(&mut saved);
        assert_eq!(saved.long("LootTableSeed"), Some(42));
        assert!(saved.string("LootTable").is_some());
        let container = entity.container_ref().expect("chest inventory");
        let guard = ContainerLockGuard::lock_all(&[&container]).expect("generate loot");
        assert!(
            guard
                .get(container.container_id())
                .expect("inventory")
                .items()
                .iter()
                .any(|item| !item.is_empty())
        );
        drop(guard);
        let mut opened = NbtCompound::new();
        entity.save_additional(&mut opened);
        assert!(opened.string("LootTable").is_none());
        assert!(opened.get("Items").is_some());
    }
}
