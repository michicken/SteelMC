//! Shared container-to-container item transfer helpers.
//!
//! These mirror the single-item move operations Vanilla hoppers and droppers perform:
//! merge into an existing compatible stack first, then use the first empty slot.

use steel_registry::item_stack::ItemStack;

use steel_registry::blocks::properties::Direction;

use crate::inventory::{
    container::Container,
    lock::{ContainerId, ContainerLockGuard},
};

/// Inserts a single item from `stack` into `dst`.
///
/// Returns `true` when exactly one item was placed.
pub(crate) fn insert_one_item(
    guard: &mut ContainerLockGuard,
    dst: ContainerId,
    stack: &ItemStack,
) -> bool {
    insert_one_item_in_slots(guard, dst, stack, None)
}

/// Inserts a single item into `dst`, restricted to `slots` when provided.
///
/// Slot restrictions are the container's automation view (`getSlotsForFace`):
/// a furnace exposes only its input slot from above, only its fuel slot from
/// the sides, and only its result slot from below.
pub(crate) fn insert_one_item_in_slots(
    guard: &mut ContainerLockGuard,
    dst: ContainerId,
    stack: &ItemStack,
    slots: Option<&[usize]>,
) -> bool {
    if stack.is_empty() {
        return false;
    }
    let Some((size, max)) = guard.get(dst).map(|container| {
        (
            container.get_container_size(),
            container.get_max_stack_size_for_item(stack),
        )
    }) else {
        return false;
    };
    if max <= 0 {
        return false;
    }
    let slot_list: Vec<usize> = slots.map_or_else(|| (0..size).collect(), <[usize]>::to_vec);

    for &slot in &slot_list {
        if slot >= size {
            continue;
        }
        let Some((existing, allowed)) = guard.get(dst).map(|container| {
            (
                container.get_item(slot).clone(),
                container.can_place_item(slot, stack),
            )
        }) else {
            return false;
        };
        if !allowed
            || existing.is_empty()
            || !ItemStack::is_same_item_same_components(&existing, stack)
            || existing.count() >= max
        {
            continue;
        }
        return guard.set_item(dst, slot, existing.copy_with_count(existing.count() + 1));
    }

    for &slot in &slot_list {
        if slot >= size {
            continue;
        }
        let Some((empty, allowed)) = guard.get(dst).map(|container| {
            (
                container.get_item(slot).is_empty(),
                container.can_place_item(slot, stack),
            )
        }) else {
            return false;
        };
        if empty && allowed {
            return guard.set_item(dst, slot, stack.copy_with_count(1));
        }
    }
    false
}

/// Moves one item from the first non-empty slot of `src` into `dst`.
pub(crate) fn move_one_item(guard: &mut ContainerLockGuard, src: ContainerId, dst: ContainerId) -> bool {
    move_one_item_in_slots(guard, src, dst, None, None, None)
}

/// Moves one item honouring the automation slot views on both sides.
pub(crate) fn move_one_item_in_slots(
    guard: &mut ContainerLockGuard,
    src: ContainerId,
    dst: ContainerId,
    src_slots: Option<&[usize]>,
    dst_slots: Option<&[usize]>,
    take_direction: Option<Direction>,
) -> bool {
    if src == dst {
        return false;
    }
    let Some(size) = guard.get(src).map(Container::get_container_size) else {
        return false;
    };
    let slot_list: Vec<usize> = src_slots.map_or_else(|| (0..size).collect(), <[usize]>::to_vec);
    for &slot in &slot_list {
        if slot >= size {
            continue;
        }
        let Some((stack, can_take)) = guard.get(src).map(|container| {
            (
                container.get_item(slot).clone(),
                take_direction.is_none_or(|direction| {
                    container.can_take_item_through_face(slot, &container.get_item(slot), direction)
                }),
            )
        }) else {
            return false;
        };
        if !can_take || stack.is_empty() || !insert_one_item_in_slots(guard, dst, &stack, dst_slots) {
            continue;
        }
        guard.remove_item(src, slot, 1);
        return true;
    }
    false
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use steel_registry::{init_vanilla_registry, item_stack::ItemStack, vanilla_items};
    use steel_utils::locks::{IntoShared as _, SyncMutex};

    use super::move_one_item;
    use crate::inventory::{
        container::{Container, SimpleContainer},
        lock::{ContainerLockGuard, ContainerRef},
    };

    const SOURCE_SLOTS: usize = 5;

    fn container_ref(size: usize) -> (Arc<SyncMutex<SimpleContainer>>, ContainerRef) {
        init_vanilla_registry();
        let container = SimpleContainer::new(size).into_shared();
        let reference = ContainerRef::from(&container);
        (container, reference)
    }

    #[test]
    fn moves_exactly_one_item_per_call() {
        let (source_container, source) = container_ref(SOURCE_SLOTS);
        let (target_container, target) = container_ref(6);
        source_container
            .lock()
            .set_item(0, ItemStack::with_count(&vanilla_items::STONE, 5));

        let mut guard = ContainerLockGuard::lock_all(&[&source, &target]).expect("locks");
        assert!(move_one_item(
            &mut guard,
            source.container_id(),
            target.container_id()
        ));
        drop(guard);

        assert_eq!(source_container.lock().get_item(0).count(), 4);
        assert_eq!(target_container.lock().get_item(0).count(), 1);
    }

    #[test]
    fn respects_max_stack_size() {
        let (source_container, source) = container_ref(SOURCE_SLOTS);
        let (target_container, target) = container_ref(1);
        source_container
            .lock()
            .set_item(0, ItemStack::with_count(&vanilla_items::STONE, 3));
        target_container
            .lock()
            .set_item(0, ItemStack::with_count(&vanilla_items::STONE, 64));

        let mut guard = ContainerLockGuard::lock_all(&[&source, &target]).expect("locks");
        assert!(!move_one_item(
            &mut guard,
            source.container_id(),
            target.container_id()
        ));
        drop(guard);

        assert_eq!(source_container.lock().get_item(0).count(), 3);
        assert_eq!(target_container.lock().get_item(0).count(), 64);
    }

    #[test]
    fn fills_empty_slots_before_stacking() {
        let (source_container, source) = container_ref(SOURCE_SLOTS);
        let (target_container, target) = container_ref(2);
        source_container
            .lock()
            .set_item(0, ItemStack::with_count(&vanilla_items::STONE, 2));
        target_container
            .lock()
            .set_item(0, ItemStack::with_count(&vanilla_items::DIRT, 1));

        let mut guard = ContainerLockGuard::lock_all(&[&source, &target]).expect("locks");
        assert!(move_one_item(
            &mut guard,
            source.container_id(),
            target.container_id()
        ));
        drop(guard);

        assert_eq!(target_container.lock().get_item(0).count(), 1);
        assert_eq!(target_container.lock().get_item(1).count(), 1);
    }
}

/// Inserts as much of `stack` into `dst` as fits, returning whatever could not be placed.
pub(crate) fn insert_stack(
    guard: &mut ContainerLockGuard,
    dst: ContainerId,
    mut stack: ItemStack,
) -> ItemStack {
    if stack.is_empty() {
        return stack;
    }
    let Some(size) = guard.get(dst).map(Container::get_container_size) else {
        return stack;
    };

    // Merge into compatible stacks first.
    for slot in 0..size {
        if stack.is_empty() {
            break;
        }
        let Some((existing, max)) = guard.get(dst).map(|container| {
            (
                container.get_item(slot).clone(),
                container.get_max_stack_size_for_item(&stack),
            )
        }) else {
            return stack;
        };
        if existing.is_empty()
            || !ItemStack::is_same_item_same_components(&existing, &stack)
            || existing.count() >= max
        {
            continue;
        }
        let moved = stack.count().min(max - existing.count());
        let taken = stack.split(moved);
        if !guard.set_item(dst, slot, existing.copy_with_count(existing.count() + taken.count())) {
            return stack;
        }
    }

    // Then fill empty slots.
    for slot in 0..size {
        if stack.is_empty() {
            break;
        }
        let Some((empty, max)) = guard.get(dst).map(|container| {
            (
                container.get_item(slot).is_empty(),
                container.get_max_stack_size_for_item(&stack),
            )
        }) else {
            return stack;
        };
        if !empty || max <= 0 {
            continue;
        }
        let moved = stack.count().min(max);
        let taken = stack.split(moved);
        if !guard.set_item(dst, slot, taken) {
            return stack;
        }
    }

    stack
}
