//! Merchant menu.
//!
//! Vanilla's merchant screen has two payment slots and two result previews.
//! Steel's settlement path lives on [`VillagerEntity::trade_with`], so the
//! result handlers validate affordability and then execute that same real
//! trade; no items are fabricated.

use std::sync::Arc;

use steel_registry::item_stack::ItemStack;
use steel_registry::vanilla_menu_types;
use steel_utils::locks::{IntoShared, Shared};

use crate::entity::Entity as _;
use crate::entity::entities::VillagerEntity;
use crate::inventory::container::{Container as _, ResultContainer, SimpleContainer};
use crate::inventory::lock::{ContainerId, ContainerLockGuard, ContainerRef};
use crate::inventory::prelude::*;
use crate::inventory::slots::ResultHandler;
use crate::player::Player;
use crate::player::player_inventory::{PlayerInventory, Prediction};

/// Builds the merchant menu for `villager`.
#[must_use]
pub fn merchant(
    inventory: Shared<PlayerInventory>,
    container_id: u8,
    villager: Arc<VillagerEntity>,
) -> Menu {
    let payment_container = SimpleContainer::new(2).into_shared();
    let payment_ref = ContainerRef::from(Arc::clone(&payment_container));

    let result_one = ResultContainer::new().into_shared();

    let mut builder = MenuBuilder::new(&vanilla_menu_types::MERCHANT, container_id);
    let payment = builder.section_all(&payment_container);
    let first = builder.result_slot(MerchantResultHandler::new(
        Arc::clone(&villager),
        0,
        Arc::clone(&payment_container),
        Arc::clone(&result_one),
    ));
    let player = builder.player_inventory(&inventory);

    builder.route(player.all(), payment, FillDirection::Forward);
    builder.route(first, player.all(), FillDirection::Backward);

    builder.build(MerchantKind {
        villager,
        payment_ref,
        payment_container,
        result_one,
    })
}

/// One result preview backed by a live villager offer.
struct MerchantResultHandler {
    villager: Arc<VillagerEntity>,
    index: usize,
    payment: Shared<SimpleContainer>,
    result: Shared<ResultContainer>,
}

impl MerchantResultHandler {
    const fn new(
        villager: Arc<VillagerEntity>,
        index: usize,
        payment: Shared<SimpleContainer>,
        result: Shared<ResultContainer>,
    ) -> Self {
        Self {
            villager,
            index,
            payment,
            result,
        }
    }

    fn offer(&self) -> Option<crate::entity::entities::MerchantOffer> {
        self.villager.trade_offers().get(self.index).cloned()
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
        match self.villager.trade_with(player, self.index) {
            Ok(_) => {
                if let Some(payment) = guard.get_mut(ContainerId::from_arc(&self.payment)) {
                    payment.set_item(0, ItemStack::empty());
                    payment.set_item(1, ItemStack::empty());
                }
                None
            }
            // The result slot was already emptied by the caller; hand it back as
            // overflow so a failed trade cannot silently eat the preview item.
            Err(_) => self.offer().map(|offer| offer.gives),
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
    villager: Arc<VillagerEntity>,
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
        _player: &Player,
    ) {
        let preview = self
            .villager
            .trade_offers()
            .first()
            .map_or_else(ItemStack::empty, |offer| offer.gives.clone());
        if let Some(container) = guard.get_mut(ContainerId::from_arc(&self.result_one)) {
            container.set_item(0, preview);
        }
    }

    fn still_valid(&self, _behavior: &MenuBehavior, _player: &Player) -> bool {
        self.villager.is_alive()
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
        let menu = merchant(Arc::clone(&player.inventory), 1, villager);
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
