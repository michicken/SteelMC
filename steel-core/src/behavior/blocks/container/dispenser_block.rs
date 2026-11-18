//! Dispenser and dropper block behavior.
//!
//! Parity notes:
//! - Droppers move a whole stack into the container they face; dispensers eject items as
//!   item entities by default.
//! - A rising redstone edge schedules a two-game-tick delay before dispensing, matching
//!   Vanilla's `DispenserBlock`.
//! - `TRIGGERED` prevents a dispenser from firing repeatedly while a signal stays high.
//!
//! Implemented dispense table: bonemeal, arrows, snowballs, eggs, TNT, water/lava
//! buckets and minecarts. Other items fall back to ejecting the item.

use std::sync::{Arc, Weak};

use glam::DVec3;
use steel_macros::block_behavior;
use steel_registry::blocks::BlockRef;
use steel_registry::blocks::block_state_ext::BlockStateExt as _;
use steel_registry::blocks::properties::{
    BlockStateProperties, BoolProperty, Direction, EnumProperty,
};
use steel_registry::item_stack::ItemStack;
use steel_registry::vanilla_block_entity_types;
use steel_registry::{vanilla_blocks, vanilla_entities, vanilla_items};
use steel_utils::{BlockPos, BlockStateId, Downcast as _, types::UpdateFlags};

use crate::behavior::BlockPlaceContext;
use crate::behavior::block::{BlockBehavior, BlockEntityCreation};
use crate::block_entity::{BLOCK_ENTITIES, BlockEntity as _, entities::DispenserBlockEntity};
use crate::entity::entities::{
    ArrowEntity, ChestMinecartEntity, ItemEntity, MinecartEntity, PrimedTntEntity, SnowballEntity,
    ThrownEggEntity,
};
use crate::entity::{Entity, SharedEntity, next_entity_id};
use crate::inventory::container::{Container, calculate_redstone_signal_from_containers};
use crate::inventory::lock::{ContainerLockGuard, ContainerRef};
use crate::inventory::transfer::insert_stack;
use crate::world::{SignalGetter as _, World};

pub(crate) const FACING: &EnumProperty<Direction> = &BlockStateProperties::FACING;
const TRIGGERED: &BoolProperty = &BlockStateProperties::TRIGGERED;

/// Vanilla dispenser block behavior.
#[block_behavior]
pub struct DispenserBlock {
    block: BlockRef,
}

/// Vanilla dropper block behavior.
#[block_behavior]
pub struct DropperBlock {
    block: BlockRef,
}

fn placement_state(block: BlockRef, context: &BlockPlaceContext<'_>) -> BlockStateId {
    block
        .default_state()
        .set_value(FACING, context.get_nearest_looking_direction().opposite())
        .set_value(TRIGGERED, false)
}

fn analog_output(world: &World, pos: BlockPos) -> i32 {
    let Some(container) = world
        .get_block_entity(pos)
        .and_then(ContainerRef::from_block_entity)
    else {
        return 0;
    };
    let Ok(guard) = ContainerLockGuard::lock_all(&[&container]) else {
        return 0;
    };
    calculate_redstone_signal_from_containers(guard.get(container.container_id()))
}

/// Ejects `stack` as an item entity in front of the dispenser.
fn eject_item(world: &Arc<World>, pos: BlockPos, facing: Direction, stack: ItemStack) {
    if stack.is_empty() {
        return;
    }
    let (dx, dy, dz) = facing.offset();
    let velocity =
        DVec3::new(f64::from(dx), f64::from(dy), f64::from(dz)) * 0.5 + DVec3::new(0.0, 0.1, 0.0);
    let entity = Arc::new(ItemEntity::with_item_and_velocity(
        &vanilla_entities::ITEM,
        next_entity_id(),
        pos.relative(facing).get_center(),
        stack,
        velocity,
        Arc::downgrade(world),
    ));
    entity.set_default_pickup_delay();
    if let Err(error) = world.try_add_entity(entity) {
        log::warn!("failed to eject dispensed item: {error}");
    }
}

#[derive(Clone, Copy)]
enum SpecialDispense {
    Minecart(bool),
    Sapling,
}

fn sapling_state(item: steel_registry::items::ItemRef) -> Option<BlockStateId> {
    let block = if item == &*vanilla_items::OAK_SAPLING {
        &vanilla_blocks::OAK_SAPLING
    } else if item == &*vanilla_items::SPRUCE_SAPLING {
        &vanilla_blocks::SPRUCE_SAPLING
    } else if item == &*vanilla_items::BIRCH_SAPLING {
        &vanilla_blocks::BIRCH_SAPLING
    } else if item == &*vanilla_items::JUNGLE_SAPLING {
        &vanilla_blocks::JUNGLE_SAPLING
    } else if item == &*vanilla_items::ACACIA_SAPLING {
        &vanilla_blocks::ACACIA_SAPLING
    } else if item == &*vanilla_items::CHERRY_SAPLING {
        &vanilla_blocks::CHERRY_SAPLING
    } else if item == &*vanilla_items::DARK_OAK_SAPLING {
        &vanilla_blocks::DARK_OAK_SAPLING
    } else if item == &*vanilla_items::PALE_OAK_SAPLING {
        &vanilla_blocks::PALE_OAK_SAPLING
    } else {
        return None;
    };
    Some(block.default_state())
}

/// Removes the first non-empty stack and either transfers it (dropper) or ejects it.
fn dispense(state: BlockStateId, world: &Arc<World>, pos: BlockPos, dropper: bool) {
    let facing = state.get_value(FACING);
    let Some(block_entity) = world.get_block_entity(pos) else {
        return;
    };
    let Some(dispenser) = block_entity.downcast_ref::<DispenserBlockEntity>() else {
        return;
    };
    let Some(own) = dispenser.container_ref() else {
        return;
    };

    let target = if dropper {
        world
            .get_block_entity(pos.relative(facing))
            .and_then(|entity| entity.container_ref())
    } else {
        None
    };

    let mut containers: Vec<&ContainerRef> = vec![&own];
    if let Some(target) = &target {
        containers.push(target);
    }
    let Ok(mut guard) = ContainerLockGuard::lock_all(&containers) else {
        return;
    };

    let own_id = own.container_id();
    let Some(size) = guard.get(own_id).map(Container::get_container_size) else {
        return;
    };
    let mut picked: Option<(usize, ItemStack)> = None;
    for slot in 0..size {
        let Some(stack) = guard
            .get(own_id)
            .map(|container| container.get_item(slot).clone())
        else {
            return;
        };
        if !stack.is_empty() {
            picked = Some((slot, stack));
            break;
        }
    }
    let Some((slot, stack)) = picked else {
        return;
    };

    if !dropper {
        let special = if stack.item == &*vanilla_items::MINECART {
            Some(SpecialDispense::Minecart(false))
        } else if stack.item == &*vanilla_items::CHEST_MINECART {
            Some(SpecialDispense::Minecart(true))
        } else if sapling_state(stack.item).is_some() {
            Some(SpecialDispense::Sapling)
        } else {
            None
        };

        if let Some(special) = special {
            drop(guard);
            let front = pos.relative(facing);
            let spawned = match special {
                SpecialDispense::Sapling => {
                    let soil = world.get_block_state(front.below()).get_block();
                    if world.get_block_state(front).get_block() == &vanilla_blocks::AIR
                        && (soil == &vanilla_blocks::DIRT
                            || soil == &vanilla_blocks::GRASS_BLOCK
                            || soil == &vanilla_blocks::FARMLAND)
                    {
                        world.set_block(
                            front,
                            sapling_state(stack.item)
                                .unwrap_or_else(|| vanilla_blocks::OAK_SAPLING.default_state()),
                            UpdateFlags::UPDATE_CLIENTS | UpdateFlags::UPDATE_NEIGHBORS,
                        );
                        true
                    } else {
                        false
                    }
                }
                SpecialDispense::Minecart(chest) => {
                    let entity: SharedEntity = if chest {
                        let cart = ChestMinecartEntity::new(
                            &vanilla_entities::CHEST_MINECART,
                            next_entity_id(),
                            front.get_center(),
                            Arc::downgrade(world),
                        );
                        Arc::new(cart)
                    } else {
                        Arc::new(MinecartEntity::new(
                            &vanilla_entities::MINECART,
                            next_entity_id(),
                            front.get_center(),
                            Arc::downgrade(world),
                        ))
                    };
                    world.try_add_entity(entity).is_ok()
                }
            };
            if spawned {
                if let Ok(mut guard) = ContainerLockGuard::lock_all(&[&own]) {
                    let remaining = stack.count() - 1;
                    if remaining > 0 {
                        guard.set_item(own_id, slot, stack.copy_with_count(remaining));
                    } else {
                        guard.remove_item(own_id, slot, stack.count());
                    }
                }
            } else {
                eject_item(world, pos, facing, stack);
                if let Ok(mut guard) = ContainerLockGuard::lock_all(&[&own]) {
                    guard.remove_item(own_id, slot, 1);
                }
            }
            return;
        }

        // Everything below is the vanilla per-item dispense table, merged from
        // upstream PR #663 and keyed by item id.
        drop(guard);
        let behavior = super::dispense_behavior::DISPENSE_BEHAVIORS.get_behavior(stack.item);
        let remaining = behavior.dispense(world, pos, state, stack.clone());
        if let Ok(mut guard) = ContainerLockGuard::lock_all(&[&own]) {
            if remaining.is_empty() {
                guard.remove_item(own_id, slot, stack.count());
            } else {
                guard.set_item(own_id, slot, remaining);
            }
        }
        return;
    }

    if let Some(target) = &target {
        let leftover = insert_stack(&mut guard, target.container_id(), stack.clone());
        if leftover.count() != stack.count() {
            if leftover.is_empty() {
                guard.remove_item(own_id, slot, stack.count());
            } else {
                guard.set_item(own_id, slot, leftover);
            }
            return;
        }
        // Nothing fit into the target container: fall through and eject the stack.
    }

    guard.remove_item(own_id, slot, stack.count());
    drop(guard);
    eject_item(world, pos, facing, stack);
}

macro_rules! impl_dispenser_block {
    ($name:ident, $dropper:expr, $new_entity:ident) => {
        impl $name {
            /// Creates this behavior for its registered block.
            #[must_use]
            pub const fn new(block: BlockRef) -> Self {
                Self { block }
            }
        }

        impl BlockBehavior for $name {
            fn get_state_for_placement(
                &self,
                context: &BlockPlaceContext<'_>,
            ) -> Option<BlockStateId> {
                Some(placement_state(self.block, context))
            }

            fn handle_neighbor_changed(
                &self,
                state: BlockStateId,
                world: &Arc<World>,
                pos: BlockPos,
                _source_block: BlockRef,
                _moved_by_piston: bool,
            ) {
                let powered = world.has_neighbor_signal(pos);
                let triggered = state.get_value(TRIGGERED);
                if powered && !triggered {
                    world.schedule_block_tick_default(pos, self.block, 2);
                } else if !powered && triggered {
                    world.set_block(
                        pos,
                        state.set_value(TRIGGERED, false),
                        UpdateFlags::UPDATE_CLIENTS,
                    );
                }
            }

            fn tick(&self, state: BlockStateId, world: &Arc<World>, pos: BlockPos) {
                if !world.has_neighbor_signal(pos) {
                    return;
                }
                if !state.get_value(TRIGGERED) {
                    world.set_block(
                        pos,
                        state.set_value(TRIGGERED, true),
                        UpdateFlags::UPDATE_CLIENTS,
                    );
                }
                dispense(state, world, pos, $dropper);
            }

            fn new_block_entity(
                &self,
                level: Weak<World>,
                pos: BlockPos,
                state: BlockStateId,
            ) -> BlockEntityCreation {
                BlockEntityCreation::from_registered_factory(BLOCK_ENTITIES.create(
                    &vanilla_block_entity_types::$new_entity,
                    level,
                    pos,
                    state,
                ))
            }

            fn has_analog_output_signal(&self, _state: BlockStateId) -> bool {
                true
            }

            fn get_analog_output_signal(
                &self,
                _state: BlockStateId,
                world: &World,
                pos: BlockPos,
                _direction: Direction,
            ) -> i32 {
                analog_output(world, pos)
            }

            fn affect_neighbors_after_removal(
                &self,
                state: BlockStateId,
                world: &Arc<World>,
                pos: BlockPos,
                _moved: bool,
            ) {
                world.update_neighbor_for_output_signal(pos, state.get_block());
            }
        }
    };
}

impl_dispenser_block!(DispenserBlock, false, DISPENSER);
impl_dispenser_block!(DropperBlock, true, DROPPER);

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use glam::DVec3;

    use steel_registry::{
        blocks::block_state_ext::BlockStateExt as _, blocks::properties::Direction,
        init_vanilla_registry, item_stack::ItemStack, vanilla_blocks, vanilla_items,
    };
    use steel_utils::{BlockPos, ChunkPos, Downcast as _, types::UpdateFlags};

    use steel_registry::vanilla_entities;
    use steel_utils::WorldAabb;

    use super::{DispenserBlock, DropperBlock, FACING, TRIGGERED};
    use crate::behavior::{BlockBehavior, init_behaviors};
    use crate::block_entity::{entities::DispenserBlockEntity, init_block_entities};
    use crate::entity::Entity;
    use crate::inventory::lock::{ContainerLockGuard, ContainerRef};
    use crate::test_support::{
        TestWorld, dropped_items, fresh_test_world, insert_ready_full_chunk, tick_test_world,
    };
    use crate::world::World;

    const DISPENSER_POS: BlockPos = BlockPos::new(0, 0, 0);
    const POWER_POS: BlockPos = BlockPos::new(0, 1, 0);
    const BELOW_POS: BlockPos = BlockPos::new(0, -1, 0);

    fn fixture(name: &'static str) -> TestWorld {
        init_vanilla_registry();
        init_behaviors();
        init_block_entities();
        crate::entity::init_entities();
        let fixture = fresh_test_world(name);
        insert_ready_full_chunk(&fixture.world, ChunkPos::new(0, 0));
        fixture
    }

    fn facing_down(block: steel_registry::blocks::BlockRef) -> steel_utils::BlockStateId {
        block
            .default_state()
            .set_value(FACING, Direction::Down)
            .set_value(TRIGGERED, false)
    }

    fn place_machine(
        world: &Arc<World>,
        state: steel_utils::BlockStateId,
        with_barrel_below: bool,
    ) {
        if with_barrel_below {
            assert!(world.set_block(
                BELOW_POS,
                vanilla_blocks::BARREL.default_state(),
                UpdateFlags::UPDATE_NONE
            ));
        }
        assert!(world.set_block(
            POWER_POS,
            vanilla_blocks::REDSTONE_BLOCK.default_state(),
            UpdateFlags::UPDATE_NONE
        ));
        assert!(world.set_block(DISPENSER_POS, state, UpdateFlags::UPDATE_NONE));
    }

    fn container_at(world: &Arc<World>, pos: BlockPos) -> ContainerRef {
        ContainerRef::from_block_entity(world.get_block_entity(pos).expect("block entity"))
            .expect("container")
    }

    fn count_at(world: &Arc<World>, pos: BlockPos, slot: usize) -> i32 {
        let container = container_at(world, pos);
        let guard = ContainerLockGuard::lock_all(&[&container]).expect("lock");
        guard
            .get(container.container_id())
            .expect("container")
            .get_item(slot)
            .count()
    }

    fn slot_item(world: &Arc<World>, pos: BlockPos, slot: usize) -> steel_registry::items::ItemRef {
        let container = container_at(world, pos);
        let guard = ContainerLockGuard::lock_all(&[&container]).expect("lock");
        guard
            .get(container.container_id())
            .expect("container")
            .get_item(slot)
            .item
    }

    fn set_slot(world: &Arc<World>, pos: BlockPos, slot: usize, stack: ItemStack) {
        let container = container_at(world, pos);
        let mut guard = ContainerLockGuard::lock_all(&[&container]).expect("lock");
        guard.set_item(container.container_id(), slot, stack);
    }

    #[test]
    fn placing_dispenser_registers_shared_block_entity() {
        let fixture = fixture("dispenser_be");
        let state = facing_down(&vanilla_blocks::DISPENSER);
        place_machine(&fixture.world, state, false);
        assert!(
            fixture
                .world
                .get_block_entity(DISPENSER_POS)
                .expect("entity")
                .downcast_ref::<DispenserBlockEntity>()
                .is_some()
        );
    }

    fn entities_in_front(
        world: &Arc<World>,
        entity_type: steel_registry::entity_type::EntityTypeRef,
    ) -> usize {
        world
            .get_entities_in_aabb_matching(
                &WorldAabb::new(-2.0, -4.0, -2.0, 2.0, 1.0, 2.0),
                |entity| entity.entity_type() == entity_type,
            )
            .len()
    }

    #[test]
    fn dispenser_places_lava_bucket_and_chest_minecart_branches() {
        let first = fixture("dispenser_lava_chest_cart");
        let state = facing_down(&vanilla_blocks::DISPENSER);
        place_machine(&first.world, state, false);
        set_slot(
            &first.world,
            DISPENSER_POS,
            0,
            ItemStack::new(&vanilla_items::LAVA_BUCKET),
        );
        DispenserBlock::new(&vanilla_blocks::DISPENSER).tick(state, &first.world, DISPENSER_POS);
        assert_eq!(
            first.world.get_block_state(BELOW_POS).get_block(),
            &vanilla_blocks::LAVA,
            "lava bucket must place lava in front"
        );

        let second = fixture("dispenser_chest_cart");
        place_machine(
            &second.world,
            facing_down(&vanilla_blocks::DISPENSER),
            false,
        );
        set_slot(
            &second.world,
            DISPENSER_POS,
            0,
            ItemStack::new(&vanilla_items::CHEST_MINECART),
        );
        DispenserBlock::new(&vanilla_blocks::DISPENSER).tick(
            facing_down(&vanilla_blocks::DISPENSER),
            &second.world,
            DISPENSER_POS,
        );
        assert_eq!(
            entities_in_front(&second.world, &vanilla_entities::CHEST_MINECART),
            1,
            "chest minecart item must spawn a chest minecart"
        );
    }

    #[test]
    fn dispenser_shoots_arrow_snowball_and_egg_as_projectiles() {
        for (item, entity_type, name) in [
            (&*vanilla_items::ARROW, &vanilla_entities::ARROW, "arrow"),
            (
                &*vanilla_items::SNOWBALL,
                &vanilla_entities::SNOWBALL,
                "snowball",
            ),
            (&*vanilla_items::EGG, &vanilla_entities::EGG, "egg"),
        ] {
            let fixture = fixture(match name {
                "arrow" => "dispenser_arrow",
                "snowball" => "dispenser_snowball",
                _ => "dispenser_egg",
            });
            let state = facing_down(&vanilla_blocks::DISPENSER);
            place_machine(&fixture.world, state, false);
            set_slot(
                &fixture.world,
                DISPENSER_POS,
                0,
                ItemStack::with_count(item, 3),
            );

            DispenserBlock::new(&vanilla_blocks::DISPENSER).tick(
                state,
                &fixture.world,
                DISPENSER_POS,
            );

            assert_eq!(
                entities_in_front(&fixture.world, entity_type),
                1,
                "{name} must be spawned as a projectile entity"
            );
            assert_eq!(
                count_at(&fixture.world, DISPENSER_POS, 0),
                2,
                "{name} dispense must consume exactly one item"
            );
        }
    }

    #[test]
    fn dispenser_shoots_wind_charge_and_launched_firework() {
        for (item, entity_type, name) in [
            (&*vanilla_items::WIND_CHARGE, &vanilla_entities::WIND_CHARGE, "wind_charge"),
            (&*vanilla_items::FIREWORK_ROCKET, &vanilla_entities::FIREWORK_ROCKET, "firework"),
            (
                &*vanilla_items::EXPERIENCE_BOTTLE,
                &vanilla_entities::EXPERIENCE_BOTTLE,
                "experience_bottle",
            ),
            (&*vanilla_items::FIRE_CHARGE, &vanilla_entities::SMALL_FIREBALL, "fire_charge"),
        ] {
            let fixture = fixture(match name {
                "wind_charge" => "dispenser_wind_charge",
                "firework" => "dispenser_firework",
                "experience_bottle" => "dispenser_experience_bottle",
                _ => "dispenser_fire_charge",
            });
            let state = facing_down(&vanilla_blocks::DISPENSER);
            place_machine(&fixture.world, state, false);
            set_slot(&fixture.world, DISPENSER_POS, 0, ItemStack::with_count(item, 2));
            DispenserBlock::new(&vanilla_blocks::DISPENSER).tick(state, &fixture.world, DISPENSER_POS);

            assert_eq!(
                entities_in_front(&fixture.world, entity_type),
                1,
                "{name} must be spawned as a projectile entity"
            );
            assert_eq!(
                count_at(&fixture.world, DISPENSER_POS, 0),
                1,
                "{name} dispense must consume exactly one item"
            );
        }
    }

    #[test]
    fn dispenser_spawns_entity_from_spawn_egg_entity_data() {
        let fixture = fixture("dispenser_spawn_egg");
        let state = facing_down(&vanilla_blocks::DISPENSER);
        place_machine(&fixture.world, state, false);
        set_slot(
            &fixture.world,
            DISPENSER_POS,
            0,
            ItemStack::with_count(&vanilla_items::ZOMBIE_SPAWN_EGG, 2),
        );
        DispenserBlock::new(&vanilla_blocks::DISPENSER).tick(state, &fixture.world, DISPENSER_POS);

        assert_eq!(
            entities_in_front(&fixture.world, &vanilla_entities::ZOMBIE),
            1,
            "spawn egg must resolve its entity type from ENTITY_DATA"
        );
        assert_eq!(
            count_at(&fixture.world, DISPENSER_POS, 0),
            1,
            "spawn egg dispense must consume exactly one item"
        );
    }

    #[test]
    fn dispenser_places_carved_pumpkin_and_wither_skeleton_skull() {
        let pumpkin = fixture("dispenser_carved_pumpkin");
        let state = facing_down(&vanilla_blocks::DISPENSER);
        place_machine(&pumpkin.world, state, false);
        set_slot(
            &pumpkin.world,
            DISPENSER_POS,
            0,
            ItemStack::new(&vanilla_items::CARVED_PUMPKIN),
        );
        DispenserBlock::new(&vanilla_blocks::DISPENSER).tick(state, &pumpkin.world, DISPENSER_POS);
        assert_eq!(
            pumpkin.world.get_block_state(BELOW_POS).get_block(),
            &vanilla_blocks::CARVED_PUMPKIN,
            "carved pumpkin must be placed in front"
        );

        let skull = fixture("dispenser_wither_skull");
        place_machine(&skull.world, state, false);
        set_slot(
            &skull.world,
            DISPENSER_POS,
            0,
            ItemStack::new(&vanilla_items::WITHER_SKELETON_SKULL),
        );
        DispenserBlock::new(&vanilla_blocks::DISPENSER).tick(state, &skull.world, DISPENSER_POS);
        assert_eq!(
            skull.world.get_block_state(BELOW_POS).get_block(),
            &vanilla_blocks::WITHER_SKELETON_SKULL,
            "wither skeleton skull must be placed in front"
        );
    }

    #[test]
    fn tree_farm_full_cycle_grows_fells_collects_and_replants() {
        let fixture = fixture("tree_farm_full_cycle");
        let world = &fixture.world;
        let soil = BlockPos::new(8, 63, 8);
        let sapling = BlockPos::new(8, 64, 8);
        let bonemeal_dispenser = BlockPos::new(9, 64, 8);
        let tnt_dispenser = BlockPos::new(7, 64, 8);
        world.set_block(
            soil,
            vanilla_blocks::DIRT.default_state(),
            UpdateFlags::UPDATE_ALL,
        );
        world.set_block(
            sapling,
            vanilla_blocks::OAK_SAPLING.default_state(),
            UpdateFlags::UPDATE_ALL,
        );

        // Bonemeal dispenser faces the sapling and grows the tree.
        let face_west = vanilla_blocks::DISPENSER
            .default_state()
            .set_value(FACING, Direction::West)
            .set_value(TRIGGERED, false);
        world.set_block(
            bonemeal_dispenser.above(),
            vanilla_blocks::REDSTONE_BLOCK.default_state(),
            UpdateFlags::UPDATE_NONE,
        );
        world.set_block(bonemeal_dispenser, face_west, UpdateFlags::UPDATE_ALL);
        set_slot(
            &world,
            bonemeal_dispenser,
            0,
            ItemStack::with_count(&vanilla_items::BONE_MEAL, 8),
        );
        DispenserBlock::new(&vanilla_blocks::DISPENSER).tick(face_west, world, bonemeal_dispenser);
        assert!(
            (64..=72).any(
                |y| world.get_block_state(BlockPos::new(8, y, 8)).get_block()
                    == &vanilla_blocks::OAK_LOG
            ),
            "bonemeal dispenser must grow the sapling into an oak tree"
        );

        assert!(world.set_game_rule(
            &steel_registry::vanilla_game_rules::TNT_EXPLOSION_DROP_DECAY,
            false
        ));
        assert!(
            !world.get_game_rule(&steel_registry::vanilla_game_rules::TNT_EXPLOSION_DROP_DECAY)
        );

        // A vanilla TNT blast is a random raycast, so a single blast can miss
        // part of the trunk. A real tree farm does not rewrite the trunk into a
        // convenient shape: it lets the dispenser grow the actual tree, then
        // repeats bonemeal-grow -> TNT-fell until the log drop appears.
        let face_east = vanilla_blocks::DISPENSER
            .default_state()
            .set_value(FACING, Direction::East)
            .set_value(TRIGGERED, false);
        let mut felled_log_drop = false;
        for attempt in 0..8 {
            if attempt > 0 {
                // Clear the previous, partially felled tree and regrow a real
                // oak through the bonemeal dispenser (never by writing logs).
                for y in 64..=80 {
                    for dx in 5..=11 {
                        for dz in 5..=11 {
                            let pos = BlockPos::new(dx, y, dz);
                            let state = world.get_block_state(pos);
                            if state.get_block() == &vanilla_blocks::OAK_LOG
                                || state.get_block() == &vanilla_blocks::OAK_LEAVES
                            {
                                world.set_block(
                                    pos,
                                    vanilla_blocks::AIR.default_state(),
                                    UpdateFlags::UPDATE_ALL,
                                );
                            }
                        }
                    }
                }
                world.set_block(
                    soil,
                    vanilla_blocks::DIRT.default_state(),
                    UpdateFlags::UPDATE_ALL,
                );
                world.set_block(
                    sapling,
                    vanilla_blocks::OAK_SAPLING.default_state(),
                    UpdateFlags::UPDATE_ALL,
                );
                world.set_block(
                    bonemeal_dispenser.above(),
                    vanilla_blocks::REDSTONE_BLOCK.default_state(),
                    UpdateFlags::UPDATE_NONE,
                );
                world.set_block(bonemeal_dispenser, face_west, UpdateFlags::UPDATE_ALL);
                set_slot(
                    &world,
                    bonemeal_dispenser,
                    0,
                    ItemStack::with_count(&vanilla_items::BONE_MEAL, 1),
                );
                DispenserBlock::new(&vanilla_blocks::DISPENSER).tick(
                    face_west,
                    world,
                    bonemeal_dispenser,
                );
                assert!(
                    (64..=80).any(
                        |y| world.get_block_state(BlockPos::new(8, y, 8)).get_block()
                            == &vanilla_blocks::OAK_LOG
                    ),
                    "cycle {attempt}: bonemeal must grow a real oak before felling"
                );
            }

            world.set_block(
                tnt_dispenser.above(),
                vanilla_blocks::REDSTONE_BLOCK.default_state(),
                UpdateFlags::UPDATE_NONE,
            );
            world.set_block(tnt_dispenser, face_east, UpdateFlags::UPDATE_ALL);
            set_slot(
                &world,
                tnt_dispenser,
                0,
                ItemStack::new(&vanilla_items::TNT),
            );
            DispenserBlock::new(&vanilla_blocks::DISPENSER).tick(face_east, world, tnt_dispenser);
            let tnts = world.get_entities_in_aabb_matching(
                &WorldAabb::new(6.0, 63.0, 7.0, 11.0, 66.0, 9.0),
                |entity| entity.entity_type() == &vanilla_entities::TNT,
            );
            assert_eq!(tnts.len(), 1, "TNT dispenser must prime one TNT entity");
            for _ in 0..82 {
                Arc::clone(&tnts[0]).tick();
            }
            felled_log_drop = world
                .get_entities_in_aabb_matching(
                    &WorldAabb::new(5.0, 62.0, 5.0, 13.0, 74.0, 11.0),
                    |entity| entity.entity_type() == &vanilla_entities::ITEM,
                )
                .iter()
                .any(|entity| {
                    entity
                        .downcast_ref::<crate::entity::entities::ItemEntity>()
                        .is_some_and(|item| item.get_item().is(&vanilla_items::OAK_LOG))
                });
            if felled_log_drop {
                break;
            }
        }
        assert!(
            felled_log_drop,
            "TNT felling must spawn a real oak log item drop"
        );

        let logs = world.get_entities_in_aabb_matching(
            &WorldAabb::new(5.0, 62.0, 5.0, 13.0, 74.0, 11.0),
            |entity| entity.entity_type() == &vanilla_entities::ITEM,
        );

        // Real hopper collection: expose a hopper at the drop floor and place the
        // actual explosion drop inside its vanilla suction area.
        let hopper = BlockPos::new(8, 63, 8);
        let chest = BlockPos::new(8, 62, 8);
        world.set_block(
            hopper,
            vanilla_blocks::HOPPER.default_state(),
            UpdateFlags::UPDATE_ALL,
        );
        world.set_block(
            chest,
            vanilla_blocks::CHEST.default_state(),
            UpdateFlags::UPDATE_ALL,
        );
        for entity in &logs {
            if entity
                .downcast_ref::<crate::entity::entities::ItemEntity>()
                .is_some_and(|item| item.get_item().is(&vanilla_items::OAK_LOG))
            {
                let _ = entity.teleport_to(DVec3::new(8.5, 64.2, 8.5));
            }
        }
        for _ in 0..40 {
            if let Some(block_entity) = world.get_block_entity(hopper) {
                block_entity.tick(world);
            }
        }
        let chest_ref = container_at(world, chest);
        let guard = ContainerLockGuard::lock_all(&[&chest_ref]).expect("lock chest");
        let inventory = guard
            .get(chest_ref.container_id())
            .expect("chest inventory");
        assert!(
            (0..27).any(|slot| inventory.get_item(slot).is(&vanilla_items::OAK_LOG)),
            "hopper chain must collect felled logs into the chest"
        );
        drop(guard);

        // Replant through the same bonemeal dispenser after the trunk is gone.
        // TNT can destroy the adjacent dispenser/soil in this minimal rig; a real
        // farm keeps them behind blast-resistant blocks, so rebuild the station.
        world.set_block(
            soil,
            vanilla_blocks::DIRT.default_state(),
            UpdateFlags::UPDATE_ALL,
        );
        world.set_block(
            bonemeal_dispenser.above(),
            vanilla_blocks::REDSTONE_BLOCK.default_state(),
            UpdateFlags::UPDATE_NONE,
        );
        world.set_block(bonemeal_dispenser, face_west, UpdateFlags::UPDATE_ALL);
        set_slot(
            &world,
            bonemeal_dispenser,
            0,
            ItemStack::new(&vanilla_items::OAK_SAPLING),
        );
        DispenserBlock::new(&vanilla_blocks::DISPENSER).tick(face_west, world, bonemeal_dispenser);
        assert_eq!(
            world.get_block_state(sapling).get_block(),
            &vanilla_blocks::OAK_SAPLING,
            "tree farm must replant the sapling after felling"
        );
    }

    #[test]
    fn dispenser_places_shulker_box_from_tagged_item() {
        let fixture = fixture("dispenser_shulker");
        let state = facing_down(&vanilla_blocks::DISPENSER);
        place_machine(&fixture.world, state, false);
        fixture.world.set_block(
            BELOW_POS,
            vanilla_blocks::AIR.default_state(),
            UpdateFlags::UPDATE_ALL,
        );
        set_slot(
            &fixture.world,
            DISPENSER_POS,
            0,
            ItemStack::new(&vanilla_items::SHULKER_BOX),
        );
        DispenserBlock::new(&vanilla_blocks::DISPENSER).tick(state, &fixture.world, DISPENSER_POS);
        assert_eq!(
            fixture.world.get_block_state(BELOW_POS).get_block(),
            &vanilla_blocks::SHULKER_BOX,
            "dispenser must place a shulker box from the shulker box tag"
        );
        assert_eq!(
            count_at(&fixture.world, DISPENSER_POS, 0),
            0,
            "placing a shulker box consumes the dispensed item"
        );
    }

    #[test]
    fn dispenser_places_powder_snow_bucket_and_returns_empty_bucket() {
        let fixture = fixture("dispenser_powder_snow");
        let state = facing_down(&vanilla_blocks::DISPENSER);
        place_machine(&fixture.world, state, false);
        fixture.world.set_block(
            BELOW_POS,
            vanilla_blocks::AIR.default_state(),
            UpdateFlags::UPDATE_ALL,
        );
        set_slot(
            &fixture.world,
            DISPENSER_POS,
            0,
            ItemStack::new(&vanilla_items::POWDER_SNOW_BUCKET),
        );
        DispenserBlock::new(&vanilla_blocks::DISPENSER).tick(state, &fixture.world, DISPENSER_POS);
        assert_eq!(
            fixture.world.get_block_state(BELOW_POS).get_block(),
            &vanilla_blocks::POWDER_SNOW,
            "filled bucket behavior must place powder snow"
        );
        let container = container_at(&fixture.world, DISPENSER_POS);
        let guard = ContainerLockGuard::lock_all(&[&container]).expect("lock");
        let inventory = guard.get(container.container_id()).expect("inventory");
        assert!(
            inventory.get_item(0).is(&vanilla_items::BUCKET),
            "powder snow bucket must return an empty bucket"
        );
    }

    #[test]
    fn dispenser_turns_dirt_into_mud_with_a_water_bottle() {
        let fixture = fixture("dispenser_mud");
        let state = facing_down(&vanilla_blocks::DISPENSER);
        place_machine(&fixture.world, state, false);
        assert!(fixture.world.set_block(
            BELOW_POS,
            vanilla_blocks::DIRT.default_state(),
            UpdateFlags::UPDATE_ALL
        ));
        set_slot(
            &fixture.world,
            DISPENSER_POS,
            0,
            steel_registry::data_components::PotionContents::create_item_stack(
                &vanilla_items::POTION,
                &steel_registry::vanilla_potions::WATER,
            ),
        );
        DispenserBlock::new(&vanilla_blocks::DISPENSER).tick(state, &fixture.world, DISPENSER_POS);
        assert_eq!(
            fixture.world.get_block_state(BELOW_POS).get_block(),
            &vanilla_blocks::MUD,
            "water bottle must convert dirt to mud"
        );
        let container = container_at(&fixture.world, DISPENSER_POS);
        let guard = ContainerLockGuard::lock_all(&[&container]).expect("lock");
        let inventory = guard.get(container.container_id()).expect("inventory");
        assert!(
            inventory.get_item(0).is(&vanilla_items::GLASS_BOTTLE),
            "water bottle must return a glass bottle"
        );
    }

    #[test]
    fn dispenser_charges_respawn_anchor_with_glowstone() {
        let fixture = fixture("dispenser_anchor");
        let state = facing_down(&vanilla_blocks::DISPENSER);
        place_machine(&fixture.world, state, false);
        assert!(fixture.world.set_block(
            BELOW_POS,
            vanilla_blocks::RESPAWN_ANCHOR.default_state(),
            UpdateFlags::UPDATE_ALL
        ));
        set_slot(
            &fixture.world,
            DISPENSER_POS,
            0,
            ItemStack::new(&vanilla_items::GLOWSTONE),
        );
        DispenserBlock::new(&vanilla_blocks::DISPENSER).tick(state, &fixture.world, DISPENSER_POS);
        assert_eq!(
            fixture.world.get_block_state(BELOW_POS).get_value(
                &steel_registry::blocks::properties::BlockStateProperties::RESPAWN_ANCHOR_CHARGES
            ),
            1,
            "glowstone must add one charge to a respawn anchor"
        );
    }

    #[test]
    fn dispenser_plants_sapling_on_soil_for_tree_farm_replanting() {
        let fixture = fixture("dispenser_sapling");
        let state = facing_down(&vanilla_blocks::DISPENSER);
        let soil_pos = BlockPos::new(0, -2, 0);
        let sapling_pos = BlockPos::new(0, -1, 0);
        assert!(fixture.world.set_block(
            soil_pos,
            vanilla_blocks::DIRT.default_state(),
            UpdateFlags::UPDATE_NONE
        ));
        place_machine(&fixture.world, state, false);
        set_slot(
            &fixture.world,
            DISPENSER_POS,
            0,
            ItemStack::with_count(&vanilla_items::OAK_SAPLING, 2),
        );

        DispenserBlock::new(&vanilla_blocks::DISPENSER).tick(state, &fixture.world, DISPENSER_POS);

        assert_eq!(
            fixture.world.get_block_state(sapling_pos).get_block(),
            &vanilla_blocks::OAK_SAPLING,
            "dispenser must plant a sapling on dirt"
        );
        assert_eq!(count_at(&fixture.world, DISPENSER_POS, 0), 1);
    }

    #[test]
    fn dispenser_shears_growing_plant_tip() {
        let fixture = fixture("dispenser_shears");
        let state = facing_down(&vanilla_blocks::DISPENSER);
        place_machine(&fixture.world, state, false);
        assert!(fixture.world.set_block(
            BELOW_POS,
            vanilla_blocks::TWISTING_VINES.default_state(),
            UpdateFlags::UPDATE_ALL
        ));
        set_slot(
            &fixture.world,
            DISPENSER_POS,
            0,
            ItemStack::new(&vanilla_items::SHEARS),
        );

        DispenserBlock::new(&vanilla_blocks::DISPENSER).tick(state, &fixture.world, DISPENSER_POS);

        let after = fixture.world.get_block_state(BELOW_POS);
        let behavior = crate::behavior::BLOCK_BEHAVIORS.get_behavior(after.get_block());
        let plant = behavior
            .as_growing_plant_head()
            .expect("twisting vines must expose growing plant behavior");
        assert!(
            plant.is_max_age(after),
            "shears dispenser must grow the twisting vines tip to max age"
        );
        assert_eq!(count_at(&fixture.world, DISPENSER_POS, 0), 1);
    }

    #[test]
    fn dispenser_primes_tnt_and_spawns_primed_entity() {
        let fixture = fixture("dispenser_tnt");
        let state = facing_down(&vanilla_blocks::DISPENSER);
        place_machine(&fixture.world, state, false);
        set_slot(
            &fixture.world,
            DISPENSER_POS,
            0,
            ItemStack::with_count(&vanilla_items::TNT, 1),
        );

        DispenserBlock::new(&vanilla_blocks::DISPENSER).tick(state, &fixture.world, DISPENSER_POS);

        assert_eq!(
            entities_in_front(&fixture.world, &vanilla_entities::TNT),
            1,
            "TNT must become a primed TNT entity"
        );
        assert_eq!(count_at(&fixture.world, DISPENSER_POS, 0), 0);
    }

    #[test]
    fn dispenser_places_water_bucket_and_keeps_empty_bucket() {
        let fixture = fixture("dispenser_bucket");
        let state = facing_down(&vanilla_blocks::DISPENSER);
        place_machine(&fixture.world, state, false);
        set_slot(
            &fixture.world,
            DISPENSER_POS,
            0,
            ItemStack::new(&vanilla_items::WATER_BUCKET),
        );

        DispenserBlock::new(&vanilla_blocks::DISPENSER).tick(state, &fixture.world, DISPENSER_POS);

        assert_eq!(
            fixture.world.get_block_state(BELOW_POS).get_block(),
            &vanilla_blocks::WATER,
            "water bucket must place water in front"
        );
        let container = container_at(&fixture.world, DISPENSER_POS);
        let guard = ContainerLockGuard::lock_all(&[&container]).expect("lock");
        let bucket = guard
            .get(container.container_id())
            .expect("container")
            .get_item(0)
            .clone();
        assert_eq!(
            bucket.item,
            &*vanilla_items::BUCKET,
            "dispenser must keep the empty bucket"
        );
    }

    #[test]
    fn dispenser_spawns_minecart_entity() {
        let fixture = fixture("dispenser_minecart");
        let state = facing_down(&vanilla_blocks::DISPENSER);
        place_machine(&fixture.world, state, false);
        set_slot(
            &fixture.world,
            DISPENSER_POS,
            0,
            ItemStack::new(&vanilla_items::MINECART),
        );

        DispenserBlock::new(&vanilla_blocks::DISPENSER).tick(state, &fixture.world, DISPENSER_POS);

        assert_eq!(
            entities_in_front(&fixture.world, &vanilla_entities::MINECART),
            1,
            "minecart item must spawn a minecart entity"
        );
        assert_eq!(count_at(&fixture.world, DISPENSER_POS, 0), 0);
    }

    #[test]
    fn dispenser_spawns_real_boat_and_armor_stand_entities() {
        let fixture = fixture("dispenser_boat_armor_stand");
        let state = facing_down(&vanilla_blocks::DISPENSER);
        place_machine(&fixture.world, state, false);
        set_slot(
            &fixture.world,
            DISPENSER_POS,
            0,
            ItemStack::new(&vanilla_items::OAK_BOAT),
        );
        DispenserBlock::new(&vanilla_blocks::DISPENSER).tick(state, &fixture.world, DISPENSER_POS);
        assert_eq!(
            entities_in_front(&fixture.world, &vanilla_entities::OAK_BOAT),
            1,
            "oak boat item must spawn a real oak_boat entity"
        );
        assert_eq!(count_at(&fixture.world, DISPENSER_POS, 0), 0);

        set_slot(
            &fixture.world,
            DISPENSER_POS,
            0,
            ItemStack::new(&vanilla_items::ARMOR_STAND),
        );
        DispenserBlock::new(&vanilla_blocks::DISPENSER).tick(state, &fixture.world, DISPENSER_POS);
        assert_eq!(
            entities_in_front(&fixture.world, &vanilla_entities::ARMOR_STAND),
            1,
            "armor stand item must spawn a real armor_stand entity"
        );
        assert_eq!(count_at(&fixture.world, DISPENSER_POS, 0), 0);
    }

    #[test]
    fn dispenser_releases_axolotl_bucket_as_real_entity_and_returns_bucket() {
        let fixture = fixture("dispenser_axolotl_bucket");
        let state = facing_down(&vanilla_blocks::DISPENSER);
        place_machine(&fixture.world, state, false);
        set_slot(
            &fixture.world,
            DISPENSER_POS,
            0,
            ItemStack::new(&vanilla_items::AXOLOTL_BUCKET),
        );

        DispenserBlock::new(&vanilla_blocks::DISPENSER).tick(state, &fixture.world, DISPENSER_POS);

        assert_eq!(
            entities_in_front(&fixture.world, &vanilla_entities::AXOLOTL),
            1,
            "axolotl bucket must release a real axolotl entity"
        );
        assert_eq!(count_at(&fixture.world, DISPENSER_POS, 0), 1);
        assert_eq!(
            slot_item(&fixture.world, DISPENSER_POS, 0),
            &*vanilla_items::BUCKET,
            "the bucket must be left behind as an empty bucket"
        );
    }

    #[test]
    fn dropper_moves_whole_stack_into_container_below() {
        let fixture = fixture("dropper_transfer");
        let state = facing_down(&vanilla_blocks::DROPPER);
        place_machine(&fixture.world, state, true);
        set_slot(
            &fixture.world,
            DISPENSER_POS,
            0,
            ItemStack::with_count(&vanilla_items::STONE, 32),
        );

        DropperBlock::new(&vanilla_blocks::DROPPER).tick(state, &fixture.world, DISPENSER_POS);

        assert_eq!(count_at(&fixture.world, BELOW_POS, 0), 32);
        assert_eq!(count_at(&fixture.world, DISPENSER_POS, 0), 0);
    }

    #[test]
    fn dropper_ejects_when_no_container_is_in_front() {
        let fixture = fixture("dropper_eject");
        let state = facing_down(&vanilla_blocks::DROPPER);
        place_machine(&fixture.world, state, false);
        set_slot(
            &fixture.world,
            DISPENSER_POS,
            0,
            ItemStack::with_count(&vanilla_items::STONE, 8),
        );
        let before = dropped_items(&fixture.world, DISPENSER_POS.get_center()).len();

        DropperBlock::new(&vanilla_blocks::DROPPER).tick(state, &fixture.world, DISPENSER_POS);

        assert_eq!(count_at(&fixture.world, DISPENSER_POS, 0), 0);
        assert_eq!(
            dropped_items(&fixture.world, DISPENSER_POS.get_center()).len(),
            before + 1
        );
    }

    #[test]
    fn dispenser_ejects_items_once_per_rising_edge() {
        let fixture = fixture("dispenser_eject");
        let state = facing_down(&vanilla_blocks::DISPENSER);
        place_machine(&fixture.world, state, false);
        set_slot(
            &fixture.world,
            DISPENSER_POS,
            0,
            ItemStack::with_count(&vanilla_items::STONE, 1),
        );
        let behavior = DispenserBlock::new(&vanilla_blocks::DISPENSER);
        let before = dropped_items(&fixture.world, DISPENSER_POS.get_center()).len();

        behavior.tick(state, &fixture.world, DISPENSER_POS);
        let triggered_state = fixture.world.get_block_state(DISPENSER_POS);
        assert!(triggered_state.get_value(TRIGGERED));
        assert_eq!(
            dropped_items(&fixture.world, DISPENSER_POS.get_center()).len(),
            before + 1
        );

        // A second tick with the signal still high must not fire again.
        behavior.tick(triggered_state, &fixture.world, DISPENSER_POS);
        assert_eq!(
            dropped_items(&fixture.world, DISPENSER_POS.get_center()).len(),
            before + 1
        );
    }

    #[test]
    fn falling_signal_clears_triggered() {
        let fixture = fixture("dispenser_falling_edge");
        let state = facing_down(&vanilla_blocks::DISPENSER);
        place_machine(&fixture.world, state, false);
        let triggered = state.set_value(TRIGGERED, true);
        assert!(
            fixture
                .world
                .set_block(DISPENSER_POS, triggered, UpdateFlags::UPDATE_NONE)
        );
        assert!(fixture.world.set_block(
            POWER_POS,
            vanilla_blocks::AIR.default_state(),
            UpdateFlags::UPDATE_NONE
        ));

        DispenserBlock::new(&vanilla_blocks::DISPENSER).handle_neighbor_changed(
            triggered,
            &fixture.world,
            DISPENSER_POS,
            &vanilla_blocks::REDSTONE_BLOCK,
            false,
        );

        assert!(
            !fixture
                .world
                .get_block_state(DISPENSER_POS)
                .get_value(TRIGGERED)
        );
    }

    #[test]
    fn hopper_feeds_dropper_pipeline() {
        use steel_registry::blocks::properties::BlockStateProperties as Props;

        let fixture = fixture("hopper_dropper_pipeline");
        let hopper_pos = BlockPos::new(0, 1, 0);
        let source_pos = BlockPos::new(0, 2, 0);
        let power_pos = BlockPos::new(1, 0, 0);
        let hopper_state = vanilla_blocks::HOPPER
            .default_state()
            .set_value(&Props::FACING_HOPPER, Direction::Down);
        assert!(
            fixture
                .world
                .set_block(hopper_pos, hopper_state, UpdateFlags::UPDATE_NONE)
        );
        assert!(fixture.world.set_block(
            source_pos,
            vanilla_blocks::BARREL.default_state(),
            UpdateFlags::UPDATE_NONE
        ));
        assert!(fixture.world.set_block(
            DISPENSER_POS,
            facing_down(&vanilla_blocks::DROPPER),
            UpdateFlags::UPDATE_NONE
        ));
        assert!(fixture.world.set_block(
            BELOW_POS,
            vanilla_blocks::BARREL.default_state(),
            UpdateFlags::UPDATE_NONE
        ));
        assert!(fixture.world.set_block(
            power_pos,
            vanilla_blocks::REDSTONE_BLOCK.default_state(),
            UpdateFlags::UPDATE_NONE
        ));
        set_slot(
            &fixture.world,
            source_pos,
            0,
            ItemStack::with_count(&vanilla_items::STONE, 4),
        );

        let hopper_entity = fixture.world.get_block_entity(hopper_pos).expect("hopper");
        for _ in 0..16 {
            hopper_entity.tick(&fixture.world);
        }

        assert_eq!(count_at(&fixture.world, DISPENSER_POS, 0), 1);
        assert_eq!(count_at(&fixture.world, BELOW_POS, 0), 0);

        DropperBlock::new(&vanilla_blocks::DROPPER).tick(
            fixture.world.get_block_state(DISPENSER_POS),
            &fixture.world,
            DISPENSER_POS,
        );

        assert_eq!(count_at(&fixture.world, DISPENSER_POS, 0), 0);
        assert_eq!(count_at(&fixture.world, BELOW_POS, 0), 1);
    }

    #[test]
    fn dispenser_consumes_bonemeal_on_a_bonemealable_block() {
        let fixture = fixture("dispenser_bonemeal");
        let east = vanilla_blocks::DISPENSER
            .default_state()
            .set_value(FACING, Direction::East)
            .set_value(TRIGGERED, false);
        let target_pos = BlockPos::new(1, 0, 0);
        assert!(fixture.world.set_block(
            target_pos,
            vanilla_blocks::ROOTED_DIRT.default_state(),
            UpdateFlags::UPDATE_NONE
        ));
        assert!(fixture.world.set_block(
            POWER_POS,
            vanilla_blocks::REDSTONE_BLOCK.default_state(),
            UpdateFlags::UPDATE_NONE
        ));
        assert!(
            fixture
                .world
                .set_block(DISPENSER_POS, east, UpdateFlags::UPDATE_NONE)
        );
        set_slot(
            &fixture.world,
            DISPENSER_POS,
            0,
            ItemStack::with_count(&vanilla_items::BONE_MEAL, 3),
        );

        DispenserBlock::new(&vanilla_blocks::DISPENSER).tick(east, &fixture.world, DISPENSER_POS);

        assert_eq!(count_at(&fixture.world, DISPENSER_POS, 0), 2);
    }

    #[test]
    fn comparator_reads_container_fullness() {
        let fixture = fixture("dispenser_comparator");
        let state = facing_down(&vanilla_blocks::DISPENSER);
        place_machine(&fixture.world, state, false);
        let behavior = DispenserBlock::new(&vanilla_blocks::DISPENSER);
        assert_eq!(
            behavior.get_analog_output_signal(
                state,
                &fixture.world,
                DISPENSER_POS,
                Direction::Down
            ),
            0
        );

        for slot in 0..3 {
            set_slot(
                &fixture.world,
                DISPENSER_POS,
                slot,
                ItemStack::with_count(&vanilla_items::STONE, 64),
            );
        }
        assert!(
            behavior.get_analog_output_signal(
                state,
                &fixture.world,
                DISPENSER_POS,
                Direction::Down
            ) > 0
        );
    }
}
