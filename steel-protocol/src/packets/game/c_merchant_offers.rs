use std::io::{Result, Write};

use steel_macros::{ClientPacket, WriteTo};
use steel_registry::item_stack::ItemStack;
use steel_registry::packets::play::C_MERCHANT_OFFERS;
use steel_utils::codec::VarInt;
use steel_utils::serial::WriteTo as WriteToTrait;

/// One vanilla merchant offer as sent to the client.
#[derive(WriteTo, Clone, Debug)]
pub struct MerchantOfferPacket {
    pub cost_a: ItemStack,
    pub cost_b: ItemStack,
    pub result: ItemStack,
    pub reward_experience: bool,
    #[write(as = VarInt)]
    pub villager_xp: i32,
    #[write(as = VarInt)]
    pub special_price: i32,
    #[write(as = VarInt)]
    pub demand: i32,
    pub price_multiplier: f32,
}

/// Vanilla `MerchantOffers` packet: the trade list for an open merchant menu.
#[derive(ClientPacket, Clone, Debug)]
#[packet_id(Play = C_MERCHANT_OFFERS)]
pub struct CMerchantOffers {
    pub container_id: i32,
    pub offers: Vec<MerchantOfferPacket>,
}

impl WriteToTrait for CMerchantOffers {
    fn write(&self, writer: &mut impl Write) -> Result<()> {
        VarInt(self.container_id).write(writer)?;
        VarInt(self.offers.len() as i32).write(writer)?;
        for offer in &self.offers {
            offer.write(writer)?;
        }
        Ok(())
    }
}
