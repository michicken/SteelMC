//! Passive entity implementations.
/// Those mobs are passive creatures that run away when attacked by a player.
mod armadillo;
mod chested_horse;
mod chicken;
mod cow;
mod goat;
mod iron_golem;
mod mooshroom;
mod pig;
mod sheep;
mod squid;
mod villager;

pub use armadillo::ArmadilloEntity;
pub use chested_horse::{
    DonkeyEntity, LlamaEntity, MuleEntity, TraderLlamaEntity,
};
pub use chicken::ChickenEntity;
pub use cow::CowEntity;
pub use goat::GoatEntity;
pub use iron_golem::IronGolemEntity;
pub use mooshroom::MushroomCowEntity;
pub use pig::PigEntity;
pub use sheep::SheepEntity;
pub use squid::SquidEntity;
pub use villager::{MerchantOffer, VillagerEntity};
