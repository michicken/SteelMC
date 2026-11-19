//! Hostile entity implementations.
//!
//! Those mobs are aggressive creatures that attack players on sight.

/// Hostile blaze entity module.
pub mod blaze;
/// Hostile endermite entity module.
pub mod endermite;
pub mod skeleton;
pub mod spider;
pub mod sulfur_cube;
pub mod zombie;
pub use blaze::BlazeEntity;
pub use endermite::EndermiteEntity;
pub use skeleton::SkeletonEntity;
pub use spider::SpiderEntity;
pub use zombie::ZombieEntity;
