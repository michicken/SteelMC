//! Grindstone result slot.
use std::sync::Arc;

use steel_registry::vanilla_enchantment_tags::EnchantmentTag;
use steel_registry::{REGISTRY, RegistryExt, TaggedRegistryExt};
use steel_registry::{item_stack::ItemStack, level_events};
use steel_utils::{BlockPos, locks::Shared};

use crate::{
    entity::entities::ExperienceOrbEntity,
    inventory::{
        container::{ResultContainer, SimpleContainer},
        prelude::*,
    },
    world::World,
};

/// Result slot handler for a grindstone.
#[derive(Clone)]
pub struct GrindstoneResultHandler {
    input_container: Shared<SimpleContainer>,
    result_container: Shared<ResultContainer>,
    block_pos: BlockPos,
    world: Arc<World>,
}

impl GrindstoneResultHandler {
    /// Creates a new handler.
    pub const fn new(
        input_container: Shared<SimpleContainer>,
        result_container: Shared<ResultContainer>,
        block_pos: BlockPos,
        world: Arc<World>,
    ) -> Self {
        Self {
            input_container,
            result_container,
            block_pos,
            world,
        }
    }

    /// Experience dropped when the result is taken: roughly half the enchanting
    /// cost of both inputs, randomized upward. Must be called before the inputs
    /// are cleared.
    fn get_experience_amount(&self, guard: &ContainerLockGuard) -> i32 {
        let Some(input) = guard.get(ContainerId::from_arc(&self.input_container)) else {
            log::warn!("input container not locked while awarding grindstone experience");
            return 0;
        };

        let amount = Self::get_experience_from_item(input.get_item(0))
            + Self::get_experience_from_item(input.get_item(1));

        if amount > 0 {
            let half_amount = Self::half_experience(amount);
            return half_amount + rand::random_range(0..half_amount);
        }
        0
    }

    /// Vanilla `GrindstoneMenu.getExperience`: half the combined enchanting
    /// cost, rounded down, clamped to at least one point when any cost exists.
    fn half_experience(total: i32) -> i32 {
        if total <= 0 {
            0
        } else {
            let half = total / 2;
            if half < 1 { 1 } else { half }
        }
    }

    fn get_experience_from_item(item: &ItemStack) -> i32 {
        let mut amount = 0;
        let Some(enchantments) = item.get_enchantments_for_crafting() else {
            return 0;
        };

        enchantments.iter().for_each(|(id, level)| {
            let Some(enchantment) = REGISTRY.enchantments.by_key(id) else {
                return;
            };

            if REGISTRY
                .enchantments
                .is_in_tag(enchantment, &EnchantmentTag::CURSE)
            {
                return;
            }

            amount += enchantment.min_cost.base
                + enchantment.min_cost.per_level_above_first * (*level as i32 - 1);
        });

        amount
    }
}

impl ResultHandler for GrindstoneResultHandler {
    fn result_container(&self) -> ContainerRef {
        ContainerRef::from(Arc::clone(&self.result_container))
    }

    fn dependencies(&self) -> Vec<ContainerRef> {
        vec![ContainerRef::from(Arc::clone(&self.input_container))]
    }

    fn update_result(&self, _guard: &mut ContainerLockGuard) {}

    fn on_result_taken(
        &self,
        guard: &mut ContainerLockGuard,
        _player: &Player,
    ) -> Option<ItemStack> {
        // Read before the inputs are cleared below.
        let experience = self.get_experience_amount(guard);
        ExperienceOrbEntity::award(&self.world, self.block_pos.get_center(), experience);

        self.world
            .level_event(level_events::SOUND_GRINDSTONE_USED, self.block_pos, 0, None);

        let id = ContainerId::from_arc(&self.input_container);
        let Some(input) = guard.get_mut(id) else {
            log::warn!("Couldn't get lock for grindstone.");
            return None;
        };

        input.set_item(0, ItemStack::empty());
        input.set_item(1, ItemStack::empty());

        None
    }

    fn is_result_valid(&self, _guard: &ContainerLockGuard, _player: &Player) -> bool {
        true
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use steel_registry::{
        init_vanilla_registry, item_stack::ItemStack, vanilla_enchantments, vanilla_items,
    };
    use steel_utils::{BlockPos, ChunkPos, locks::IntoShared as _};

    use super::GrindstoneResultHandler;
    use crate::{
        behavior::init_behaviors,
        inventory::{
            container::{Container as _, ResultContainer, SimpleContainer},
            lock::{ContainerId, ContainerLockGuard, ContainerRef},
            slots::result_handler::ResultHandler,
        },
        test_support::{TestPlayerBuilder, fresh_test_world, insert_ready_full_chunk},
    };

    #[test]
    fn half_experience_rounds_down_with_one_point_floor() {
        // 0 costs grant no experience; any positive cost grants at least one.
        assert_eq!(GrindstoneResultHandler::half_experience(0), 0);
        assert_eq!(GrindstoneResultHandler::half_experience(1), 1);
        assert_eq!(GrindstoneResultHandler::half_experience(2), 1);
        // Odd totals round down (vanilla floorDiv), not up.
        assert_eq!(GrindstoneResultHandler::half_experience(3), 1);
        assert_eq!(GrindstoneResultHandler::half_experience(4), 2);
        assert_eq!(GrindstoneResultHandler::half_experience(5), 2);
        assert_eq!(GrindstoneResultHandler::half_experience(6), 3);
        assert_eq!(GrindstoneResultHandler::half_experience(7), 3);
    }

    #[test]
    fn curse_enchantments_do_not_grindstone_experience() {
        init_vanilla_registry();

        let mut curse_book = ItemStack::new(&vanilla_items::ENCHANTED_BOOK);
        curse_book.set_enchantments(
            &[(vanilla_enchantments::VANISHING_CURSE.key.clone(), 1)],
            false,
        );

        assert_eq!(
            GrindstoneResultHandler::get_experience_from_item(&curse_book),
            0
        );
    }

    #[test]
    fn normal_enchantment_counts_toward_grindstone_experience() {
        init_vanilla_registry();

        let mut book = ItemStack::new(&vanilla_items::ENCHANTED_BOOK);
        book.set_enchantments(
            &[(vanilla_enchantments::SHARPNESS.key.clone(), 1)],
            false,
        );

        assert!(GrindstoneResultHandler::get_experience_from_item(&book) > 0);
    }

    #[test]
    fn taking_the_grindstone_result_clears_both_inputs() {
        init_vanilla_registry();
        init_behaviors();

        let world_fixture = fresh_test_world("grindstone_clears_both_inputs");
        let world = &world_fixture.world;
        let pos = BlockPos::new(0, 64, 0);
        insert_ready_full_chunk(world, ChunkPos::from_block_pos(pos));
        let player = TestPlayerBuilder::new(Arc::clone(world), "Grinder", 1).build();

        let input = SimpleContainer::new(2).into_shared();
        let result = ResultContainer::new().into_shared();
        let handler = GrindstoneResultHandler::new(
            Arc::clone(&input),
            Arc::clone(&result),
            pos,
            Arc::clone(world),
        );

        let mut book = ItemStack::new(&vanilla_items::ENCHANTED_BOOK);
        book.set_enchantments(
            &[(vanilla_enchantments::SHARPNESS.key.clone(), 1)],
            false,
        );
        {
            let mut locked = input.lock();
            locked.set_item(0, book.clone());
            locked.set_item(1, book);
        }

        let input_ref = ContainerRef::from(&input);
        let mut guard = ContainerLockGuard::lock_all(&[input_ref])
            .expect("grindstone input container should lock");
        handler.on_result_taken(&mut guard, &player);

        let after = guard.get(ContainerId::from_arc(&input)).expect("input locked");
        assert!(after.get_item(0).is_empty());
        assert!(after.get_item(1).is_empty());
    }
}
