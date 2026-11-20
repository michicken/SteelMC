//! Hostile entity implementations.
//!
//! Those mobs are aggressive creatures that attack players on sight.

/// Hostile blaze entity module.
pub mod blaze;
/// Hostile endermite entity module.
pub mod enderman;
pub mod endermite;
pub mod ghast;
pub mod piglin;
pub mod skeleton;
pub mod spider;
pub mod sulfur_cube;
pub mod wither_skeleton;
pub mod zombie;
pub use blaze::BlazeEntity;
pub use enderman::EndermanEntity;
pub use endermite::EndermiteEntity;
pub use ghast::GhastEntity;
pub use piglin::PiglinEntity;
pub use skeleton::SkeletonEntity;
pub use spider::SpiderEntity;
pub use wither_skeleton::WitherSkeletonEntity;
pub use zombie::ZombieEntity;
