//! Fuse and explosion behavior shared by all TNT minecart creation paths.

use std::sync::{Arc, Weak};

use simdnbt::{borrow::NbtCompound as BorrowedNbtCompoundView, owned::NbtCompound};
use steel_protocol::packets::game::SoundSource;
use steel_registry::blocks::block_state_ext::BlockStateExt as _;
use steel_registry::sound_events;
use steel_registry::vanilla_block_tags::BlockTag;
use steel_registry::vanilla_damage_type_tags::DamageTypeTag;
use steel_registry::vanilla_entities;
use steel_registry::vanilla_game_rules::TNT_EXPLODES;
use steel_utils::entity_events::EntityStatus;
use steel_utils::locks::SyncMutex;
use steel_utils::nbt::NbtValueInput as _;
use steel_utils::{BlockPos, BlockStateId};

use super::minecart::MinecartEntity;
use crate::entity::{DamageSource, Entity, RemovalReason, SharedEntity, WeakEntity};
use crate::world::{ExplosionInteraction, ExplosionOptions, World};

const UNPRIMED_FUSE: i32 = -1;
const FUSE_TICKS: i32 = 80;
const CHAIN_FUSE_ROLL_TICKS: i32 = 20;
const DEFAULT_POWER: f32 = 4.0;
const DEFAULT_SPEED_FACTOR: f32 = 1.0;
const MAX_POWER_COMPONENT: f32 = 128.0;
const IMPACT_SPEED_SQUARED: f64 = 0.01_f32 as f64;
const MIN_EXPLOSIVE_FALL_DISTANCE: f64 = 3.0;
const FALL_DISTANCE_PER_SPEED_UNIT: f64 = 10.0;
const MAX_SPEED_BONUS_INPUT: f64 = 5.0;
const SPEED_POWER_MULTIPLIER: f64 = 1.5;

pub(super) struct TntMinecart {
    state: SyncMutex<TntMinecartState>,
}

struct TntMinecartState {
    fuse: i32,
    power: f32,
    speed_factor: f32,
    ignition_cause: Option<WeakEntity>,
}

impl TntMinecart {
    pub(super) fn new() -> Self {
        Self {
            state: SyncMutex::new(TntMinecartState {
                fuse: UNPRIMED_FUSE,
                power: DEFAULT_POWER,
                speed_factor: DEFAULT_SPEED_FACTOR,
                ignition_cause: None,
            }),
        }
    }

    pub(super) fn prime(
        &self,
        cart: &MinecartEntity,
        world: &World,
        source: Option<&DamageSource>,
    ) {
        if !world.get_game_rule(&TNT_EXPLODES) {
            return;
        }
        {
            let mut state = self.state.lock();
            if state.fuse >= 0 {
                return;
            }
            state.fuse = if source.is_some() {
                rand::random_range(0..CHAIN_FUSE_ROLL_TICKS)
                    + rand::random_range(0..CHAIN_FUSE_ROLL_TICKS)
            } else {
                FUSE_TICKS
            };
            state.ignition_cause = source
                .and_then(DamageSource::causing_entity)
                .map(Arc::downgrade);
        }
        cart.broadcast_entity_event(EntityStatus::TntPrime);
        if !cart.is_silent() {
            world.play_sound_at(
                &sound_events::ENTITY_TNT_PRIMED,
                SoundSource::Blocks,
                cart.position(),
                1.0,
                1.0,
                None,
            );
        }
    }

    pub(super) fn damage_ignites(source: &DamageSource) -> bool {
        if let Some(direct) = source.direct_entity()
            && direct.as_projectile().is_some()
        {
            return direct.is_on_fire();
        }
        source.is(&DamageTypeTag::IS_FIRE) || source.is(&DamageTypeTag::IS_EXPLOSION)
    }

    pub(super) fn handle_destroy(
        &self,
        cart: &MinecartEntity,
        world: &World,
        source: &DamageSource,
    ) -> bool {
        if !Self::damage_ignites(source)
            && Self::horizontal_speed_squared(cart) < IMPACT_SPEED_SQUARED
        {
            return false;
        }
        self.prime(cart, world, Some(source));
        true
    }

    pub(super) fn handle_projectile_hit(
        &self,
        cart: &MinecartEntity,
        world: &World,
        source: &DamageSource,
    ) {
        if let Some(projectile) = source.direct_entity()
            && (projectile.entity_type() == &vanilla_entities::ARROW
                || projectile.entity_type() == &vanilla_entities::SPECTRAL_ARROW
                || projectile.entity_type() == &vanilla_entities::TRIDENT)
            && projectile.is_on_fire()
            && let Some(entity) = world.get_entity_by_id(cart.id())
        {
            self.explode(
                &entity,
                projectile.velocity().length_squared(),
                Some(source),
            );
        }
    }

    pub(super) fn tick(&self, cart: &MinecartEntity, entity: &SharedEntity) {
        let expired = {
            let mut state = self.state.lock();
            let expired = state.fuse == 0;
            if state.fuse > 0 {
                state.fuse -= 1;
            }
            expired
        };
        let speed_squared = Self::horizontal_speed_squared(cart);
        if expired || (cart.horizontal_collision() && speed_squared >= IMPACT_SPEED_SQUARED) {
            self.explode(entity, speed_squared, None);
        }
    }

    pub(super) fn handle_fall(&self, cart: &Arc<MinecartEntity>, distance: f64) {
        if distance >= MIN_EXPLOSIVE_FALL_DISTANCE {
            let speed = distance / FALL_DISTANCE_PER_SPEED_UNIT;
            let entity: SharedEntity = Arc::<MinecartEntity>::clone(cart);
            self.explode(&entity, speed * speed, None);
        }
    }

    fn horizontal_speed_squared(cart: &MinecartEntity) -> f64 {
        let motion = cart.velocity();
        motion.x * motion.x + motion.z * motion.z
    }

    fn explode(
        &self,
        cart: &SharedEntity,
        speed_squared: f64,
        projectile_source: Option<&DamageSource>,
    ) {
        if cart.is_removed() {
            return;
        }
        let Some(world) = cart.level() else {
            return;
        };
        let state = self.state.lock();
        if !world.get_game_rule(&TNT_EXPLODES) {
            let primed = state.fuse >= 0;
            drop(state);
            if primed {
                cart.set_removed(RemovalReason::Discarded);
            }
            return;
        }
        let speed = speed_squared.sqrt().min(MAX_SPEED_BONUS_INPUT);
        let power = f64::from(state.power)
            + f64::from(state.speed_factor)
                * rand::random::<f64>()
                * SPEED_POWER_MULTIPLIER
                * speed;
        let ignition = projectile_source
            .map(|source| source.causing_entity().map(Arc::clone))
            .or_else(|| state.ignition_cause.as_ref().map(Weak::upgrade));
        let damage_source =
            ignition.map(|cause| DamageSource::explosion(Some(Arc::clone(cart)), cause));
        drop(state);
        let mut options =
            ExplosionOptions::new(cart.position(), power as f32, ExplosionInteraction::Tnt);
        options.source = Some(Arc::clone(cart));
        options.damage_source = damage_source;
        world.explode(options);
        cart.set_removed(RemovalReason::Discarded);
    }

    pub(super) fn protects_rail(&self, world: &World, pos: BlockPos, state: BlockStateId) -> bool {
        self.state.lock().fuse >= 0
            && (state.get_block().has_tag(&BlockTag::RAILS)
                || world
                    .get_block_state(pos.above())
                    .get_block()
                    .has_tag(&BlockTag::RAILS))
    }

    pub(super) fn save(&self, nbt: &mut NbtCompound) {
        let state = self.state.lock();
        nbt.insert("fuse", state.fuse);
        if state.power != DEFAULT_POWER {
            nbt.insert("explosion_power", state.power);
        }
        if state.speed_factor != DEFAULT_SPEED_FACTOR {
            nbt.insert("explosion_speed_factor", state.speed_factor);
        }
    }

    pub(super) fn load(&self, nbt: BorrowedNbtCompoundView<'_, '_>) {
        let mut state = self.state.lock();
        state.fuse = nbt.int_value("fuse").unwrap_or(UNPRIMED_FUSE);
        state.power = nbt
            .float_value("explosion_power")
            .unwrap_or(DEFAULT_POWER)
            .clamp(0.0, MAX_POWER_COMPONENT);
        state.speed_factor = nbt
            .float_value("explosion_speed_factor")
            .unwrap_or(DEFAULT_SPEED_FACTOR)
            .clamp(0.0, MAX_POWER_COMPONENT);
        state.ignition_cause = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::entity::ENTITIES;
    use crate::entity::entities::objects::vehicles::test_support::test_world;
    use glam::DVec3;
    use steel_utils::ChunkPos;

    #[test]
    fn summoned_tnt_minecart_primes_and_explodes_after_the_fuse() {
        let fixture = test_world("summoned_tnt_minecart", ChunkPos::new(0, 0));
        let world = &fixture.world;
        let cart = crate::entity::create_entity_instance(
            world,
            &vanilla_entities::TNT_MINECART,
            DVec3::new(8.5, 64.0, 8.5),
        )
        .expect("TNT minecart has a registered factory");
        assert!(ENTITIES.has_factory(&vanilla_entities::TNT_MINECART));
        world
            .try_add_entity(Arc::clone(&cart))
            .expect("loaded cart chunk");
        let minecart = cart.as_abstract_minecart().expect("minecart capability");
        assert!(!minecart.is_rideable());

        minecart.activate_minecart(world, cart.block_position(), false);
        let mut saved = NbtCompound::new();
        cart.save_additional(&mut saved);
        assert_eq!(saved.int("fuse"), Some(-1));

        minecart.activate_minecart(world, cart.block_position(), true);
        for _ in 0..FUSE_TICKS {
            Arc::clone(&cart).tick();
        }
        assert!(!cart.is_removed());
        Arc::clone(&cart).tick();
        assert!(cart.is_removed());
    }
}
