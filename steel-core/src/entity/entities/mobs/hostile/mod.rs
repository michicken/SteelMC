//! Hostile entity implementations.
//!
//! Those mobs are aggressive creatures that attack players on sight.

/// Hostile endermite entity module.
pub mod endermite;
pub mod skeleton;
pub mod spider;
pub mod zombie;
pub use endermite::EndermiteEntity;
pub use skeleton::SkeletonEntity;
pub use spider::SpiderEntity;
pub use zombie::ZombieEntity;
