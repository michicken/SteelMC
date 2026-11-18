//! Villager merchant offers and trade settlement.
//!
//! The offer table is generated from the vanilla `villager_trade` data pack
//! (level 1 offers).  Each offer stores the exact wanted stacks and the exact
//! result stack, plus `max_uses`, `uses` and the XP granted per trade.  Trade
//! settlement mutates the real player inventory; it does not fabricate items.

use steel_registry::item_stack::ItemStack;
use steel_registry::vanilla_items;

use crate::inventory::container::Container as _;
use crate::player::Player;
use crate::player::player_inventory::PlayerInventory;

/// One vanilla merchant offer.
#[derive(Debug, Clone)]
pub struct MerchantOffer {
    /// Stacks the player must hand over.
    pub wants: Vec<ItemStack>,
    /// Stack the player receives.
    pub gives: ItemStack,
    pub max_uses: i32,
    pub uses: i32,
    pub xp: i32,
}

impl MerchantOffer {
    /// Returns true when the offer has been exhausted.
    #[must_use]
    pub const fn is_disabled(&self) -> bool {
        self.uses >= self.max_uses
    }
}

/// Why a trade could not be completed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TradeError {
    NoSuchOffer,
    OfferDisabled,
    MissingInputs,
    OutputDoesNotFit,
}

/// Result of a successful trade.
#[derive(Debug, Clone, PartialEq)]
pub struct TradeOutcome {
    pub xp: i32,
    pub level_ups: i32,
}

const fn offer(wants: Vec<ItemStack>, gives: ItemStack, max_uses: i32, xp: i32) -> MerchantOffer {
    MerchantOffer {
        wants,
        gives,
        max_uses,
        uses: 0,
        xp,
    }
}

/// Builds all vanilla offers unlocked at `level` for a profession key.
///
/// Level 1 is the hand-written exact table; levels 2-5 are appended from the
/// generated vanilla `villager_trade` data. Offers whose output is produced by
/// a dynamic item modifier (e.g. random enchanted books) are intentionally
/// omitted because they are not exact stacks.
#[must_use]
pub fn offers_for_level(path: &str, level: i32) -> Vec<MerchantOffer> {
    let mut offers = level_one_offers(path);
    if level > 1 {
        super::trade_table::append_level_offers(path, level, &mut offers);
    }
    offers
}

/// Builds the vanilla level-1 offer list for a profession key.
///
/// `path` is the profession registry path (`farmer`, `librarian`, ...).
#[must_use]
pub fn level_one_offers(path: &str) -> Vec<MerchantOffer> {
    match path {
        "armorer" => vec![
            offer(
                vec![ItemStack::with_count(&vanilla_items::EMERALD, 4)],
                ItemStack::with_count(&vanilla_items::IRON_BOOTS, 1),
                12,
                0,
            ),
            offer(
                vec![ItemStack::with_count(&vanilla_items::EMERALD, 9)],
                ItemStack::with_count(&vanilla_items::IRON_CHESTPLATE, 1),
                12,
                0,
            ),
            offer(
                vec![ItemStack::with_count(&vanilla_items::EMERALD, 5)],
                ItemStack::with_count(&vanilla_items::IRON_HELMET, 1),
                12,
                0,
            ),
            offer(
                vec![ItemStack::with_count(&vanilla_items::EMERALD, 7)],
                ItemStack::with_count(&vanilla_items::IRON_LEGGINGS, 1),
                12,
                0,
            ),
        ],
        "butcher" => vec![
            offer(
                vec![ItemStack::with_count(&vanilla_items::CHICKEN, 14)],
                ItemStack::with_count(&vanilla_items::EMERALD, 1),
                16,
                2,
            ),
            offer(
                vec![ItemStack::with_count(&vanilla_items::EMERALD, 1)],
                ItemStack::with_count(&vanilla_items::RABBIT_STEW, 1),
                12,
                0,
            ),
            offer(
                vec![ItemStack::with_count(&vanilla_items::PORKCHOP, 7)],
                ItemStack::with_count(&vanilla_items::EMERALD, 1),
                16,
                2,
            ),
            offer(
                vec![ItemStack::with_count(&vanilla_items::RABBIT, 4)],
                ItemStack::with_count(&vanilla_items::EMERALD, 1),
                16,
                2,
            ),
        ],
        "cartographer" => vec![
            offer(
                vec![ItemStack::with_count(&vanilla_items::EMERALD, 7)],
                ItemStack::with_count(&vanilla_items::MAP, 1),
                12,
                0,
            ),
            offer(
                vec![ItemStack::with_count(&vanilla_items::PAPER, 24)],
                ItemStack::with_count(&vanilla_items::EMERALD, 1),
                12,
                2,
            ),
        ],
        "cleric" => vec![
            offer(
                vec![ItemStack::with_count(&vanilla_items::EMERALD, 1)],
                ItemStack::with_count(&vanilla_items::REDSTONE, 2),
                12,
                0,
            ),
            offer(
                vec![ItemStack::with_count(&vanilla_items::ROTTEN_FLESH, 32)],
                ItemStack::with_count(&vanilla_items::EMERALD, 1),
                16,
                2,
            ),
        ],
        "farmer" => vec![
            offer(
                vec![ItemStack::with_count(&vanilla_items::BEETROOT, 15)],
                ItemStack::with_count(&vanilla_items::EMERALD, 1),
                16,
                2,
            ),
            offer(
                vec![ItemStack::with_count(&vanilla_items::CARROT, 22)],
                ItemStack::with_count(&vanilla_items::EMERALD, 1),
                16,
                2,
            ),
            offer(
                vec![ItemStack::with_count(&vanilla_items::EMERALD, 1)],
                ItemStack::with_count(&vanilla_items::BREAD, 6),
                16,
                0,
            ),
            offer(
                vec![ItemStack::with_count(&vanilla_items::POTATO, 26)],
                ItemStack::with_count(&vanilla_items::EMERALD, 1),
                16,
                2,
            ),
            offer(
                vec![ItemStack::with_count(&vanilla_items::WHEAT, 20)],
                ItemStack::with_count(&vanilla_items::EMERALD, 1),
                16,
                2,
            ),
        ],
        "fisherman" => vec![
            offer(
                vec![ItemStack::with_count(&vanilla_items::COAL, 10)],
                ItemStack::with_count(&vanilla_items::EMERALD, 1),
                16,
                2,
            ),
            offer(
                vec![ItemStack::with_count(&vanilla_items::EMERALD, 3)],
                ItemStack::with_count(&vanilla_items::COD_BUCKET, 1),
                16,
                0,
            ),
            offer(
                vec![
                    ItemStack::with_count(&vanilla_items::COD, 6),
                    ItemStack::with_count(&vanilla_items::EMERALD, 1),
                ],
                ItemStack::with_count(&vanilla_items::COOKED_COD, 6),
                16,
                0,
            ),
            offer(
                vec![ItemStack::with_count(&vanilla_items::STRING, 20)],
                ItemStack::with_count(&vanilla_items::EMERALD, 1),
                16,
                2,
            ),
        ],
        "fletcher" => vec![
            offer(
                vec![ItemStack::with_count(&vanilla_items::EMERALD, 1)],
                ItemStack::with_count(&vanilla_items::ARROW, 16),
                12,
                0,
            ),
            offer(
                vec![
                    ItemStack::with_count(&vanilla_items::GRAVEL, 10),
                    ItemStack::with_count(&vanilla_items::EMERALD, 1),
                ],
                ItemStack::with_count(&vanilla_items::FLINT, 10),
                12,
                0,
            ),
            offer(
                vec![ItemStack::with_count(&vanilla_items::STICK, 32)],
                ItemStack::with_count(&vanilla_items::EMERALD, 1),
                16,
                2,
            ),
        ],
        "leatherworker" => vec![offer(
            vec![ItemStack::with_count(&vanilla_items::LEATHER, 6)],
            ItemStack::with_count(&vanilla_items::EMERALD, 1),
            16,
            2,
        )],
        "librarian" => vec![
            offer(
                vec![ItemStack::with_count(&vanilla_items::EMERALD, 9)],
                ItemStack::with_count(&vanilla_items::BOOKSHELF, 1),
                12,
                0,
            ),
            offer(
                vec![ItemStack::with_count(&vanilla_items::PAPER, 24)],
                ItemStack::with_count(&vanilla_items::EMERALD, 1),
                16,
                2,
            ),
        ],
        "mason" => vec![
            offer(
                vec![ItemStack::with_count(&vanilla_items::CLAY_BALL, 10)],
                ItemStack::with_count(&vanilla_items::EMERALD, 1),
                16,
                2,
            ),
            offer(
                vec![ItemStack::with_count(&vanilla_items::EMERALD, 1)],
                ItemStack::with_count(&vanilla_items::BRICK, 10),
                16,
                0,
            ),
        ],
        "shepherd" => vec![
            offer(
                vec![ItemStack::with_count(&vanilla_items::BLACK_WOOL, 18)],
                ItemStack::with_count(&vanilla_items::EMERALD, 1),
                16,
                2,
            ),
            offer(
                vec![ItemStack::with_count(&vanilla_items::BROWN_WOOL, 18)],
                ItemStack::with_count(&vanilla_items::EMERALD, 1),
                16,
                2,
            ),
            offer(
                vec![ItemStack::with_count(&vanilla_items::EMERALD, 2)],
                ItemStack::with_count(&vanilla_items::SHEARS, 1),
                12,
                0,
            ),
            offer(
                vec![ItemStack::with_count(&vanilla_items::GRAY_WOOL, 18)],
                ItemStack::with_count(&vanilla_items::EMERALD, 1),
                16,
                2,
            ),
            offer(
                vec![ItemStack::with_count(&vanilla_items::WHITE_WOOL, 18)],
                ItemStack::with_count(&vanilla_items::EMERALD, 1),
                16,
                2,
            ),
        ],
        "toolsmith" => vec![
            offer(
                vec![ItemStack::with_count(&vanilla_items::EMERALD, 1)],
                ItemStack::with_count(&vanilla_items::STONE_AXE, 1),
                12,
                0,
            ),
            offer(
                vec![ItemStack::with_count(&vanilla_items::EMERALD, 1)],
                ItemStack::with_count(&vanilla_items::STONE_HOE, 1),
                12,
                0,
            ),
            offer(
                vec![ItemStack::with_count(&vanilla_items::EMERALD, 1)],
                ItemStack::with_count(&vanilla_items::STONE_PICKAXE, 1),
                12,
                0,
            ),
            offer(
                vec![ItemStack::with_count(&vanilla_items::EMERALD, 1)],
                ItemStack::with_count(&vanilla_items::STONE_SHOVEL, 1),
                12,
                0,
            ),
        ],
        "weaponsmith" => vec![offer(
            vec![ItemStack::with_count(&vanilla_items::EMERALD, 3)],
            ItemStack::with_count(&vanilla_items::IRON_AXE, 1),
            12,
            0,
        )],
        _ => Vec::new(),
    }
}

fn count_matching(inventory: &PlayerInventory, wanted: &ItemStack) -> i32 {
    inventory
        .items()
        .iter()
        .filter(|stack| stack.item() == wanted.item())
        .map(ItemStack::count)
        .sum()
}

fn remove_matching(inventory: &mut PlayerInventory, wanted: &ItemStack, count: i32) {
    let mut remaining = count;
    for slot in 0..inventory.get_container_size() {
        if remaining <= 0 {
            break;
        }
        let stack = inventory.get_item(slot);
        if stack.is_empty() || stack.item() != wanted.item() {
            continue;
        }
        let take = stack.count().min(remaining);
        let mut stack = stack.clone();
        stack.shrink(take);
        inventory.set_item(slot, stack);
        remaining -= take;
    }
}

/// Attempts to execute offer `index` for `player`.
///
/// This is the server-side settlement path: inputs are removed from the real
/// inventory, the output is inserted into it, the offer's `uses` counter is
/// incremented and trade XP is awarded.  GUI/menu plumbing can call this
/// method once a client selects an offer.
pub fn execute_trade(
    offers: &mut [MerchantOffer],
    player: &Player,
    index: usize,
) -> Result<TradeOutcome, TradeError> {
    let offer = offers.get_mut(index).ok_or(TradeError::NoSuchOffer)?;
    if offer.is_disabled() {
        return Err(TradeError::OfferDisabled);
    }

    let mut inventory = player.inventory.lock();
    if offer
        .wants
        .iter()
        .any(|wanted| count_matching(&inventory, wanted) < wanted.count())
    {
        return Err(TradeError::MissingInputs);
    }

    for wanted in &offer.wants {
        remove_matching(&mut inventory, wanted, wanted.count());
    }

    let mut result = offer.gives.clone();
    if !inventory.add(&mut result) || !result.is_empty() {
        // Put the inputs back, otherwise a full inventory would eat them.
        for wanted in &offer.wants {
            let mut refund = wanted.clone();
            let _ = inventory.add(&mut refund);
        }
        return Err(TradeError::OutputDoesNotFit);
    }

    offer.uses += 1;
    Ok(TradeOutcome {
        xp: offer.xp,
        level_ups: 0,
    })
}

/// Vanilla villager trade XP thresholds: level 1 starts at 0, then 2, 10, 30,
/// 70 cumulative XP for levels 2-5.
#[must_use]
pub fn level_for_xp(xp: i32) -> i32 {
    match xp {
        i32::MIN..=1 => 1,
        2..=9 => 2,
        10..=29 => 3,
        30..=69 => 4,
        _ => 5,
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use steel_registry::{init_vanilla_registry, item_stack::ItemStack, vanilla_items};

    use super::{TradeError, execute_trade, level_for_xp, level_one_offers, offers_for_level};

    #[test]
    fn farmer_offer_table_matches_vanilla_level_one() {
        init_vanilla_registry();
        let offers = level_one_offers("farmer");
        assert_eq!(offers.len(), 5);
        assert_eq!(offers[4].wants[0].item(), &*vanilla_items::WHEAT);
        assert_eq!(offers[4].wants[0].count(), 20);
        assert_eq!(offers[4].gives.item(), &*vanilla_items::EMERALD);
        assert_eq!(offers[4].max_uses, 16);
        assert_eq!(offers[4].xp, 2);
    }

    #[test]
    fn level_thresholds_match_vanilla() {
        assert_eq!(level_for_xp(0), 1);
        assert_eq!(level_for_xp(2), 2);
        assert_eq!(level_for_xp(10), 3);
        assert_eq!(level_for_xp(30), 4);
        assert_eq!(level_for_xp(70), 5);
    }

    #[test]
    fn farmer_level_two_unlocks_vanilla_sixteen_xp_offers() {
        steel_registry::init_vanilla_registry();
        let level_one = level_one_offers("farmer");
        let level_two = offers_for_level("farmer", 2);
        assert!(
            level_two.len() > level_one.len(),
            "level 2 must append the vanilla farmer offers"
        );
        assert!(
            level_two.iter().any(|offer| {
                offer.gives.is(&vanilla_items::APPLE)
                    && offer.gives.count() == 4
                    && offer.wants.iter().any(|want| want.is(&vanilla_items::EMERALD))
            }),
            "farmer level 2 must include 1 emerald -> 4 apples"
        );
    }

    #[test]
    fn farmer_trade_moves_real_items_and_exhausts_the_offer() {
        use crate::inventory::container::Container as _;
        use crate::test_support::{TestPlayerBuilder, fresh_test_world};

        init_vanilla_registry();
        let fixture = fresh_test_world("merchant_real_trade");
        let player = TestPlayerBuilder::new(Arc::clone(&fixture.world), "Trader", 1).build();
        player
            .inventory
            .lock()
            .set_item(0, ItemStack::with_count(&vanilla_items::WHEAT, 20));

        let mut offers = level_one_offers("farmer");
        let outcome = execute_trade(&mut offers, &player, 4).expect("wheat trade must succeed");
        assert_eq!(outcome.xp, 2);
        assert_eq!(offers[4].uses, 1);

        let inventory = player.inventory.lock();
        let wheat: i32 = inventory
            .items()
            .iter()
            .filter(|stack| stack.item() == &*vanilla_items::WHEAT)
            .map(ItemStack::count)
            .sum();
        assert_eq!(wheat, 0, "20 wheat must be consumed");
        let emeralds: i32 = inventory
            .items()
            .iter()
            .filter(|stack| stack.item() == &*vanilla_items::EMERALD)
            .map(ItemStack::count)
            .sum();
        assert_eq!(emeralds, 1, "trade must insert the real emerald result");
    }

    #[test]
    fn disabled_offer_is_rejected_before_consuming_inputs() {
        use crate::inventory::container::Container as _;
        use crate::test_support::{TestPlayerBuilder, fresh_test_world};

        init_vanilla_registry();
        let fixture = fresh_test_world("merchant_disabled_offer");
        let player = TestPlayerBuilder::new(Arc::clone(&fixture.world), "Trader2", 1).build();
        player
            .inventory
            .lock()
            .set_item(0, ItemStack::with_count(&vanilla_items::WHEAT, 20));

        let mut offers = level_one_offers("farmer");
        offers[4].uses = offers[4].max_uses;
        assert_eq!(
            execute_trade(&mut offers, &player, 4),
            Err(TradeError::OfferDisabled)
        );
        assert_eq!(
            player.inventory.lock().get_item(0).count(),
            20,
            "a disabled offer must not eat the inputs"
        );
    }
}
