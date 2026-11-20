//! Vanilla observer behavior.

use std::sync::Arc;

use steel_macros::block_behavior;
use steel_registry::blocks::BlockRef;
use steel_registry::blocks::block_state_ext::BlockStateExt as _;
use steel_registry::blocks::properties::{
    BlockStateProperties, BoolProperty, Direction, EnumProperty,
};
use steel_utils::types::UpdateFlags;
use steel_utils::{BlockPos, BlockStateId};

use crate::behavior::blocks::redstone::{MAX_REDSTONE_SIGNAL, MIN_REDSTONE_SIGNAL};
use crate::behavior::{BlockBehavior, BlockPlaceContext};
use crate::world::{LevelReader, ScheduledTickAccess, SignalQueryContext, World};

const PULSE_DELAY: i32 = 2;
const PLACEMENT_RESET_FLAGS: UpdateFlags =
    UpdateFlags::UPDATE_CLIENTS.union(UpdateFlags::UPDATE_KNOWN_SHAPE);

/// Vanilla `ObserverBlock` two-game-tick pulse behavior.
#[block_behavior]
pub struct ObserverBlock {
    block: BlockRef,
}

const FACING: &EnumProperty<Direction> = &BlockStateProperties::FACING;
const POWERED: &BoolProperty = &BlockStateProperties::POWERED;

impl ObserverBlock {
    /// Creates observer behavior for `block`.
    #[must_use]
    pub const fn new(block: BlockRef) -> Self {
        Self { block }
    }

    fn start_signal(
        &self,
        ticks: &dyn ScheduledTickAccess,
        pos: BlockPos,
        _state: BlockStateId,
    ) {
        if !ticks.has_scheduled_block_tick(pos, self.block) {
            ticks.schedule_block_tick_default(pos, self.block, PULSE_DELAY);
        }
    }

    fn update_neighbors_in_front(&self, world: &Arc<World>, pos: BlockPos, state: BlockStateId) {
        let direction = state.get_value(FACING);
        let output_pos = pos.relative(direction.opposite());
        world.neighbor_changed(output_pos, self.block);
        world.update_neighbors_at_except_from_facing(output_pos, self.block, direction);
    }

    fn own_signal(state: BlockStateId) -> i32 {
        if state.get_value(POWERED) {
            MAX_REDSTONE_SIGNAL
        } else {
            MIN_REDSTONE_SIGNAL
        }
    }
}

impl BlockBehavior for ObserverBlock {
    fn get_state_for_placement(&self, context: &BlockPlaceContext<'_>) -> Option<BlockStateId> {
        Some(
            self.block
                .default_state()
                .set_value(FACING, context.get_nearest_looking_direction()),
        )
    }

    fn update_shape(
        &self,
        state: BlockStateId,
        world: &dyn ScheduledTickAccess,
        pos: BlockPos,
        direction: Direction,
        _neighbor_pos: BlockPos,
        _neighbor_state: BlockStateId,
    ) -> BlockStateId {
        if state.get_value(FACING) == direction && !state.get_value(POWERED) {
            self.start_signal(world, pos, state);
        }
        state
    }

    fn on_neighbor_updated(
        &self,
        state: BlockStateId,
        world: &Arc<World>,
        pos: BlockPos,
        from_pos: BlockPos,
        _source_block: BlockRef,
        _moved_by_piston: bool,
    ) {
        // Only react to a change on the watched face; neighbor updates from
        // the other five sides must not schedule a pulse. The scheduled-tick
        // and powered guards are shared with `update_shape` to avoid double
        // scheduling while a pulse is already armed.
        if state.get_value(POWERED) {
            return;
        }
        if from_pos != pos.relative(state.get_value(FACING)) {
            return;
        }
        self.start_signal(world, pos, state);
    }

    fn tick(&self, state: BlockStateId, world: &Arc<World>, pos: BlockPos) {
        if state.get_value(POWERED) {
            world.set_block(
                pos,
                state.set_value(POWERED, false),
                UpdateFlags::UPDATE_CLIENTS,
            );
        } else {
            world.set_block(
                pos,
                state.set_value(POWERED, true),
                UpdateFlags::UPDATE_CLIENTS,
            );
            world.schedule_block_tick_default(pos, self.block, PULSE_DELAY);
        }
        self.update_neighbors_in_front(world, pos, state);
        // Vanilla observers also let the block they are watching see the
        // powered-state change; without this two observers facing each other
        // pulse once and then stall instead of forming a clock.
        world.update_neighbor_shapes_at(
            state.set_value(POWERED, !state.get_value(POWERED)),
            pos,
            UpdateFlags::UPDATE_CLIENTS,
            0,
        );
    }

    fn on_place(
        &self,
        state: BlockStateId,
        world: &Arc<World>,
        pos: BlockPos,
        old_state: BlockStateId,
        moved_by_piston: bool,
    ) {
        // Vanilla observers re-arm when a piston moves them: the block they
        // were watching is no longer in the same place, so they must emit a
        // fresh pulse after settling. Flying-machine engines depend on this.
        // A powered observer that is moved mid-pulse must still deliver its
        // turn-off tick at the destination. The scheduled tick stays behind on
        // the old position, so re-arm the off phase here.
        if moved_by_piston
            && state.get_value(POWERED)
            && !world.has_scheduled_block_tick(pos, self.block)
        {
            world.schedule_block_tick_default(pos, self.block, 0);
        } else {
            let _ = moved_by_piston;
        }

        if state.get_block() == old_state.get_block()
            || !state.get_value(POWERED)
            || world.has_scheduled_block_tick(pos, self.block)
        {
            return;
        }

        let reset_state = state.set_value(POWERED, false);
        world.set_block(pos, reset_state, PLACEMENT_RESET_FLAGS);
        self.update_neighbors_in_front(world, pos, reset_state);
    }

    fn affect_neighbors_after_removal(
        &self,
        state: BlockStateId,
        world: &Arc<World>,
        pos: BlockPos,
        _moved_by_piston: bool,
    ) {
        if state.get_value(POWERED) && world.has_scheduled_block_tick(pos, self.block) {
            self.update_neighbors_in_front(world, pos, state.set_value(POWERED, false));
        }
    }

    fn is_signal_source(&self, _state: BlockStateId, _context: SignalQueryContext) -> bool {
        true
    }

    fn get_own_signal(
        &self,
        state: BlockStateId,
        _world: &dyn LevelReader,
        _pos: BlockPos,
        _context: SignalQueryContext,
    ) -> i32 {
        Self::own_signal(state)
    }

    fn get_signal(
        &self,
        state: BlockStateId,
        _world: &dyn LevelReader,
        _pos: BlockPos,
        direction: Direction,
        _context: SignalQueryContext,
    ) -> i32 {
        if state.get_value(FACING) == direction {
            Self::own_signal(state)
        } else {
            0
        }
    }

    fn get_direct_signal(
        &self,
        state: BlockStateId,
        world: &dyn LevelReader,
        pos: BlockPos,
        direction: Direction,
        context: SignalQueryContext,
    ) -> i32 {
        self.get_signal(state, world, pos, direction, context)
    }
}

#[cfg(test)]
mod tests {
    use steel_registry::init_vanilla_registry;
    use steel_registry::vanilla_blocks;

    use super::*;
    use crate::test_support::TestLevel;

    fn observer_state(facing: Direction, powered: bool) -> BlockStateId {
        vanilla_blocks::OBSERVER
            .default_state()
            .set_value(FACING, facing)
            .set_value(POWERED, powered)
    }

    #[test]
    fn observed_face_update_schedules_one_two_tick_pulse() {
        init_vanilla_registry();
        let observer = ObserverBlock::new(&vanilla_blocks::OBSERVER);
        let pos = BlockPos::new(0, 64, 0);
        let state = observer_state(Direction::East, false);
        let level = TestLevel::default();

        observer.update_shape(
            state,
            &level,
            pos,
            Direction::East,
            pos.east(),
            vanilla_blocks::STONE.default_state(),
        );
        let scheduled = level.scheduled_block_ticks.borrow();
        assert_eq!(scheduled.len(), 1);
        assert_eq!(scheduled[0].pos, pos);
        assert_eq!(scheduled[0].block, &vanilla_blocks::OBSERVER);
        assert_eq!(scheduled[0].delay, PULSE_DELAY);
    }

    #[test]
    fn observer_output_is_powered_and_directional() {
        init_vanilla_registry();
        let observer = ObserverBlock::new(&vanilla_blocks::OBSERVER);
        let state = observer_state(Direction::Down, true);
        let level = TestLevel::default();
        let pos = BlockPos::new(0, 64, 0);

        assert_eq!(
            observer.get_signal(
                state,
                &level,
                pos,
                Direction::Down,
                SignalQueryContext::DEFAULT,
            ),
            15
        );
        assert_eq!(
            observer.get_signal(
                state,
                &level,
                pos,
                Direction::Up,
                SignalQueryContext::DEFAULT,
            ),
            0
        );
    }

    #[test]
    fn two_facing_observers_form_a_real_world_clock() {
        use std::sync::Arc;

        use steel_utils::ChunkPos;
        use steel_utils::types::UpdateFlags;

        use crate::test_support::{
            fresh_test_world, insert_ready_full_chunk, tick_test_world,
        };

        init_vanilla_registry();
        crate::behavior::init_behaviors();

        // Both axes must oscillate; a clock that only works west/east would be a
        // directional phase hack rather than vanilla observer behavior.
        for (name, a, b, facing_a, facing_b) in [
            (
                "observer_pair_clock_ew",
                BlockPos::new(8, 64, 8),
                BlockPos::new(9, 64, 8),
                Direction::East,
                Direction::West,
            ),
            (
                "observer_pair_clock_ns",
                BlockPos::new(8, 64, 8),
                BlockPos::new(8, 64, 9),
                Direction::South,
                Direction::North,
            ),
        ] {
            let fixture = fresh_test_world(name);
            let world = &fixture.world;
            insert_ready_full_chunk(world, ChunkPos::new(0, 0));
            let state_a = observer_state(facing_a, false);
            let state_b = observer_state(facing_b, false);
            world.set_block(a, state_a, UpdateFlags::UPDATE_ALL);
            world.set_block(b, state_b, UpdateFlags::UPDATE_ALL);
            let observer = ObserverBlock::new(&vanilla_blocks::OBSERVER);
            observer.update_shape(state_a, world, a, facing_a, b, state_b);
            observer.update_shape(state_b, world, b, facing_b, a, state_a);

            let mut last = world.get_block_state(a).get_value(POWERED);
            let mut flips = 0;
            for tick in 0..200_u64 {
                tick_test_world(world, tick, true);
                let now = world.get_block_state(a).get_value(POWERED);
                if now != last {
                    flips += 1;
                    last = now;
                }
            }
            assert!(
                flips >= 8,
                "{name}: two facing observers must keep toggling for 200gt, got {flips} flips"
            );
        }
    }

    #[test]
    fn observer_pulses_once_on_front_container_neighbor_update() {
        use std::sync::Arc;

        use steel_utils::ChunkPos;
        use steel_utils::types::UpdateFlags;

        use crate::test_support::{
            fresh_test_world, insert_ready_full_chunk, tick_test_world,
        };

        init_vanilla_registry();
        crate::behavior::init_behaviors();
        crate::block_entity::init_block_entities();

        let fixture = fresh_test_world("observer_front_container");
        let world: &Arc<World> = &fixture.world;
        insert_ready_full_chunk(world, ChunkPos::new(0, 0));

        let observer_pos = BlockPos::new(8, 64, 8);
        let front_pos = observer_pos.east();
        // Place the watched container first so its placement shape updates
        // cannot arm the observer; the observer is installed afterwards.
        world.set_block(
            front_pos,
            vanilla_blocks::CHEST.default_state(),
            UpdateFlags::UPDATE_NONE,
        );
        world.set_block(
            observer_pos,
            observer_state(Direction::East, false),
            UpdateFlags::UPDATE_NONE,
        );
        assert!(
            !world.has_scheduled_block_tick(observer_pos, &vanilla_blocks::OBSERVER),
            "setup must not already arm the observer"
        );

        // The chest's inventory changed but its block state did not: only a
        // neighbor update originates from the watched face.
        world.update_neighbors_at(front_pos, &vanilla_blocks::CHEST);

        let mut last = world.get_block_state(observer_pos).get_value(POWERED);
        let mut pulses = 0;
        for _ in 0..8 {
            tick_test_world(world, 1, true);
            let now = world.get_block_state(observer_pos).get_value(POWERED);
            if now && !last {
                pulses += 1;
            }
            last = now;
        }
        assert_eq!(
            pulses, 1,
            "a neighbor update from the watched container must produce exactly one pulse"
        );
    }

    #[test]
    fn observer_ignores_neighbor_updates_from_behind_and_side() {
        use std::sync::Arc;

        use steel_utils::ChunkPos;
        use steel_utils::types::UpdateFlags;

        use crate::test_support::{
            fresh_test_world, insert_ready_full_chunk, tick_test_world,
        };

        init_vanilla_registry();
        crate::behavior::init_behaviors();
        crate::block_entity::init_block_entities();

        let fixture = fresh_test_world("observer_rear_side_updates");
        let world: &Arc<World> = &fixture.world;
        insert_ready_full_chunk(world, ChunkPos::new(0, 0));

        let observer_pos = BlockPos::new(8, 64, 8);
        let behind_pos = observer_pos.west();
        let side_pos = observer_pos.north();
        world.set_block(behind_pos, vanilla_blocks::STONE.default_state(), UpdateFlags::UPDATE_NONE);
        world.set_block(side_pos, vanilla_blocks::STONE.default_state(), UpdateFlags::UPDATE_NONE);
        world.set_block(
            observer_pos,
            observer_state(Direction::East, false),
            UpdateFlags::UPDATE_NONE,
        );
        assert!(
            !world.has_scheduled_block_tick(observer_pos, &vanilla_blocks::OBSERVER),
            "setup must not already arm the observer"
        );

        // Updates from the rear face and the side face must not arm the observer.
        world.update_neighbors_at(behind_pos, &vanilla_blocks::STONE);
        world.update_neighbors_at(side_pos, &vanilla_blocks::STONE);

        let mut last = world.get_block_state(observer_pos).get_value(POWERED);
        let mut pulses = 0;
        for _ in 0..8 {
            tick_test_world(world, 1, true);
            let now = world.get_block_state(observer_pos).get_value(POWERED);
            if now && !last {
                pulses += 1;
            }
            last = now;
        }
        assert_eq!(
            pulses, 0,
            "neighbor updates from non-watched faces must not pulse the observer"
        );
    }
}
