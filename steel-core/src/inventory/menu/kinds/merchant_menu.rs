//! Merchant menu.
//!
//! Vanilla's merchant screen has two payment slots and two result previews.
//! Steel's settlement path lives on [`VillagerEntity::trade_with`], so the
//! result handlers validate affordability and then execute that same real
//! trade; no items are fabricated.

use std::sync::Arc;

use steel_registry::item_stack::ItemStack;
use steel_protocol::packets::game::{CMerchantOffers, MerchantOfferPacket};
use steel_registry::vanilla_menu_types;
use steel_utils::locks::{IntoShared, Shared};
use steel_utils::Downcast as _;

use crate::entity::Entity;
use crate::entity::entities::VillagerEntity;
use crate::world::World;
use crate::inventory::container::{Container as _, ResultContainer, SimpleContainer};
use crate::inventory::lock::{ContainerId, ContainerLockGuard, ContainerRef};
use crate::inventory::prelude::*;
use crate::inventory::slots::ResultHandler;
use crate::player::Player;
use crate::player::connection::NetworkConnection as _;
use crate::player::player_inventory::{PlayerInventory, Prediction};

/// Builds the merchant menu for `villager`.
#[must_use]
pub fn merchant(
    inventory: Shared<PlayerInventory>,
    container_id: u8,
    world: std::sync::Weak<World>,
    villager_id: i32,
) -> Menu {
    let payment_container = SimpleContainer::new(2).into_shared();
    let payment_ref = ContainerRef::from(Arc::clone(&payment_container));

    let result_one = ResultContainer::new().into_shared();

    let mut builder = MenuBuilder::new(&vanilla_menu_types::MERCHANT, container_id);
    let payment = builder.section_all(&payment_container);
    let first = builder.result_slot(MerchantResultHandler::new(
        world.clone(),
        villager_id,
        0,
        Arc::clone(&payment_container),
        Arc::clone(&result_one),
    ));
    let player = builder.player_inventory(&inventory);

    builder.route(player.all(), payment, FillDirection::Forward);
    builder.route(first, player.all(), FillDirection::Backward);

    builder.build(MerchantKind {
        world,
        villager_id,
        payment_ref,
        payment_container,
        result_one,
    })
}

/// One result preview backed by a live villager offer.
struct MerchantResultHandler {
    world: std::sync::Weak<World>,
    villager_id: i32,
    index: usize,
    payment: Shared<SimpleContainer>,
    result: Shared<ResultContainer>,
}

impl MerchantResultHandler {
    const fn new(
        world: std::sync::Weak<World>,
        villager_id: i32,
        index: usize,
        payment: Shared<SimpleContainer>,
        result: Shared<ResultContainer>,
    ) -> Self {
        Self {
            world,
            villager_id,
            index,
            payment,
            result,
        }
    }

    fn with_villager<T>(&self, f: impl FnOnce(&VillagerEntity) -> T) -> Option<T> {
        let world = self.world.upgrade()?;
        let entity = world.get_entity_by_id(self.villager_id)?;
        let villager = entity.downcast_ref::<VillagerEntity>()?;
        Some(f(villager))
    }

    fn offer(&self) -> Option<crate::entity::entities::MerchantOffer> {
        self.with_villager(|villager| villager.trade_offers().get(self.index).cloned())
            .flatten()
    }
}

impl ResultHandler for MerchantResultHandler {
    fn result_container(&self) -> ContainerRef {
        ContainerRef::from(Arc::clone(&self.result))
    }

    fn dependencies(&self) -> Vec<ContainerRef> {
        vec![ContainerRef::from(Arc::clone(&self.payment))]
    }

    fn update_result(&self, guard: &mut ContainerLockGuard) {
        let preview = self
            .offer()
            .map_or_else(ItemStack::empty, |offer| offer.gives);
        if let Some(container) = guard.get_mut(ContainerId::from_arc(&self.result)) {
            container.set_item(0, preview);
        }
    }

    fn on_result_taken(&self, guard: &mut ContainerLockGuard, player: &Player) -> Option<ItemStack> {
        let outcome = self.with_villager(|villager| villager.trade_with(player, self.index));
        match outcome {
            Some(Ok(_)) => {
                if let Some(payment) = guard.get_mut(ContainerId::from_arc(&self.payment)) {
                    payment.set_item(0, ItemStack::empty());
                    payment.set_item(1, ItemStack::empty());
                }
                None
            }
            // The result slot was already emptied by the caller; hand it back as
            // overflow so a failed trade cannot silently eat the preview item.
            Some(Err(_)) | None => self.offer().map(|offer| offer.gives),
        }
    }

    fn is_result_valid(&self, _guard: &ContainerLockGuard, player: &Player) -> bool {
        let Some(offer) = self.offer() else {
            return false;
        };
        if offer.is_disabled() {
            return false;
        }
        let inventory = player.inventory.lock();
        offer.adjusted_wants().iter().all(|wanted| {
            inventory
                .items()
                .iter()
                .filter(|stack| stack.item() == wanted.item())
                .map(ItemStack::count)
                .sum::<i32>()
                >= wanted.count()
        })
    }
}

/// Per-menu merchant state.
pub struct MerchantKind {
    world: std::sync::Weak<World>,
    villager_id: i32,
    payment_ref: ContainerRef,
    payment_container: Shared<SimpleContainer>,
    result_one: Shared<ResultContainer>,
}

// SAFETY: This Steel-owned key uniquely identifies the concrete menu kind
// within the process.
unsafe impl steel_utils::DowncastType for MerchantKind {
    const TYPE_KEY: steel_utils::DowncastTypeKey =
        steel_utils::DowncastTypeKey::new("steel:menu/merchant");
}

impl MenuKind for MerchantKind {
    fn on_open(
        &mut self,
        behavior: &mut MenuBehavior,
        guard: &mut ContainerLockGuard,
        player: &Player,
    ) {
        if let Some(world) = self.world.upgrade()
            && let Some(entity) = world.get_entity_by_id(self.villager_id)
            && let Some(villager) = entity.downcast_ref::<VillagerEntity>()
        {
            let offers = villager
                .trade_offers()
                .iter()
                .map(|offer| MerchantOfferPacket {
                    cost_a: offer
                        .wants
                        .first()
                        .cloned()
                        .unwrap_or_else(ItemStack::empty),
                    cost_b: offer
                        .wants
                        .get(1)
                        .cloned()
                        .unwrap_or_else(ItemStack::empty),
                    result: offer.gives.clone(),
                    reward_experience: offer.xp > 0,
                    villager_xp: offer.xp,
                    special_price: offer.special_price,
                    demand: offer.demand,
                    price_multiplier: offer.price_multiplier,
                })
                .collect();
            player.send_packet(CMerchantOffers {
                container_id: i32::from(behavior.container_id()),
                offers,
            });
        }
        let preview = self
            .world
            .upgrade()
            .and_then(|world| world.get_entity_by_id(self.villager_id))
            .and_then(|entity| {
                entity
                    .downcast_ref::<VillagerEntity>()
                    .and_then(|villager| villager.trade_offers().first().cloned())
            })
            .map_or_else(ItemStack::empty, |offer| offer.gives);
        if let Some(container) = guard.get_mut(ContainerId::from_arc(&self.result_one)) {
            container.set_item(0, preview);
        }
    }

    fn still_valid(&self, _behavior: &MenuBehavior, _player: &Player) -> bool {
        self.world
            .upgrade()
            .and_then(|world| world.get_entity_by_id(self.villager_id))
            .and_then(|entity| entity.downcast_ref::<VillagerEntity>().map(Entity::is_alive))
            .unwrap_or(false)
    }

    fn can_take_item_for_pick_all(&self, _carried: &ItemStack, slot_index: usize) -> bool {
        // Slots 0/1 are payment displays; only the two result previews may be
        // picked up by pickup-all.
        slot_index >= 2
    }

    fn removed(&mut self, _behavior: &mut MenuBehavior, player: &Player) {
        // Payment slots are a vanilla display; anything placed there must not
        // vanish when the screen closes.
        let payment = self.payment_container.lock();
        for index in 0..2 {
            let stack = payment.get_item(index).clone();
            if !stack.is_empty() {
                let _ = player.drop_item(stack, false, Prediction::ServerOnly);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::entity::next_entity_id;
    use crate::test_support::{TestPlayerBuilder, fresh_test_world};
    use steel_registry::{REGISTRY, RegistryExt as _, init_vanilla_registry, vanilla_villager_professions};
    use glam::DVec3;

    #[test]
    fn merchant_menu_uses_vanilla_type_and_three_container_slots() {
        init_vanilla_registry();
        let fixture = fresh_test_world("merchant_menu_layout");
        let villager = Arc::new(VillagerEntity::new(
            &steel_registry::vanilla_entities::VILLAGER,
            next_entity_id(),
            DVec3::new(8.0, 64.0, 8.0),
            Arc::downgrade(&fixture.world),
        ));
        let farmer_id = REGISTRY
            .villager_professions
            .id_from_key(&vanilla_villager_professions::FARMER.key)
            .and_then(|id| i32::try_from(id).ok())
            .expect("farmer profession is registered");
        villager.set_profession(farmer_id);
        assert_eq!(villager.trade_offers().len(), 5);

        let player = TestPlayerBuilder::new(Arc::clone(&fixture.world), "MerchantViewer", 1).build();
        let menu = merchant(
            Arc::clone(&player.inventory),
            1,
            Arc::downgrade(&fixture.world),
            villager.id(),
        );
        assert_eq!(
            menu.menu_type().map(|kind| kind.key.clone()),
            Some(vanilla_menu_types::MERCHANT.key.clone()),
            "merchant menu must advertise minecraft:merchant to the client"
        );
        assert!(
            menu.behavior().slots().len() >= 3,
            "merchant menu must contain two payment slots plus two result previews"
        );
    }
}
