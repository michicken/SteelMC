use std::io::{Result, Write};

use steel_macros::ClientPacket;
use steel_registry::RegistryEntry as _;
use steel_registry::item_stack::ItemStack;
use steel_registry::packets::play::C_MERCHANT_OFFERS;
use steel_utils::codec::VarInt;
use steel_utils::serial::WriteTo as WriteToTrait;

/// One vanilla merchant offer as sent to the client.
///
/// This mirrors vanilla's `MerchantOffer.writeToStream` order exactly. In
/// particular, the result precedes the optional second cost, and the numeric
/// counters are fixed-width `int`s rather than VarInts.
#[derive(Clone, Debug)]
pub struct MerchantOfferPacket {
    pub cost_a: ItemStack,
    pub result: ItemStack,
    pub cost_b: Option<ItemStack>,
    pub out_of_stock: bool,
    pub uses: i32,
    pub max_uses: i32,
    pub xp: i32,
    pub special_price: i32,
    pub price_multiplier: f32,
    pub demand: i32,
}

/// Writes a vanilla `ItemCost`: registry item id, count, then an empty
/// `DataComponentExactPredicate`. Steel's generated vanilla trade costs carry
/// no component predicate; item components on the result use `ItemStack` below.
fn write_item_cost(stack: &ItemStack, writer: &mut impl Write) -> Result<()> {
    let item_id = i32::try_from(stack.item().id())
        .map_err(|_| std::io::Error::other("merchant cost item id is too large"))?;
    VarInt(item_id).write(writer)?;
    VarInt(stack.count()).write(writer)?;
    // DataComponentExactPredicate::EMPTY is a zero-length component list.
    VarInt(0).write(writer)?;
    Ok(())
}

impl WriteToTrait for MerchantOfferPacket {
    fn write(&self, writer: &mut impl Write) -> Result<()> {
        write_item_cost(&self.cost_a, writer)?;
        self.result.write(writer)?;
        if let Some(cost_b) = &self.cost_b {
            writer.write_all(&[1])?;
            write_item_cost(cost_b, writer)?;
        } else {
            writer.write_all(&[0])?;
        }
        writer.write_all(&[u8::from(self.out_of_stock)])?;
        writer.write_all(&self.uses.to_be_bytes())?;
        writer.write_all(&self.max_uses.to_be_bytes())?;
        writer.write_all(&self.xp.to_be_bytes())?;
        writer.write_all(&self.special_price.to_be_bytes())?;
        writer.write_all(&self.price_multiplier.to_be_bytes())?;
        writer.write_all(&self.demand.to_be_bytes())?;
        Ok(())
    }
}

/// Vanilla `MerchantOffers` packet: the trade list for an open merchant menu.
#[derive(ClientPacket, Clone, Debug)]
#[packet_id(Play = C_MERCHANT_OFFERS)]
pub struct CMerchantOffers {
    pub container_id: i32,
    pub offers: Vec<MerchantOfferPacket>,
    pub villager_level: i32,
    pub villager_xp: i32,
    pub show_progress: bool,
    pub can_restock: bool,
}

impl WriteToTrait for CMerchantOffers {
    fn write(&self, writer: &mut impl Write) -> Result<()> {
        VarInt(self.container_id).write(writer)?;
        VarInt(self.offers.len() as i32).write(writer)?;
        for offer in &self.offers {
            offer.write(writer)?;
        }
        VarInt(self.villager_level).write(writer)?;
        VarInt(self.villager_xp).write(writer)?;
        writer.write_all(&[u8::from(self.show_progress)])?;
        writer.write_all(&[u8::from(self.can_restock)])?;
        Ok(())
    }
}
