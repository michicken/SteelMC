//! Passive entity implementations.
/// Those mobs are passive creatures that run away when attacked by a player.
mod armadillo;
mod chicken;
mod cow;
mod iron_golem;
mod mooshroom;
mod pig;
mod sheep;
mod squid;
mod villager;

pub use armadillo::ArmadilloEntity;
pub use chicken::ChickenEntity;
pub use cow::CowEntity;
pub use iron_golem::IronGolemEntity;
pub use mooshroom::MushroomCowEntity;
pub use pig::PigEntity;
pub use sheep::SheepEntity;
pub use squid::SquidEntity;
pub use villager::VillagerEntity;
