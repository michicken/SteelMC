//! Vehicle entity implementations.

mod abstract_minecart;
mod boat;
mod chest_boat;
mod chest_minecart;
mod minecart;
mod minecart_behavior;
mod spawner_minecart;
mod tnt_minecart;
mod vehicle_entity;

pub use abstract_minecart::{AbstractMinecart, AbstractMinecartBase};
pub use boat::BoatEntity;
pub use chest_boat::ChestBoatEntity;
pub(crate) use abstract_minecart::{CreateMinecartError, create_minecart};
pub use chest_minecart::ChestMinecartEntity;
pub use minecart::MinecartEntity;
pub use minecart_behavior::MinecartBehavior;
pub use spawner_minecart::SpawnerMinecartEntity;
pub use vehicle_entity::VehicleEntity;

#[cfg(test)]
#[path = "tests/support.rs"]
mod test_support;
