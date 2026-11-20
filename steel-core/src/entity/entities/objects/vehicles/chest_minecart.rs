//! Chest minecart uses the shared minecart implementation.

/// Chest minecarts use the minecart entity with chest-specific state.
pub type ChestMinecartEntity = super::MinecartEntity;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::entity::Entity;
    use glam::DVec3;
    use simdnbt::owned::NbtCompound;
    use std::sync::Weak;
    use steel_registry::vanilla_entities;
    use steel_utils::Identifier;
    use steel_utils::axis::Axis;
    use steel_utils::block_util::FoundRectangle;

    #[test]
    fn chest_minecart_saves_structure_loot_table_state() {
        let minecart = ChestMinecartEntity::new(
            &vanilla_entities::CHEST_MINECART,
            1,
            DVec3::new(1.5, 2.5, 3.5),
            Weak::new(),
        );
        minecart.set_loot_table(
            Identifier::new_static("minecraft", "chests/abandoned_mineshaft"),
            42,
        );

        let mut nbt = NbtCompound::new();
        minecart.save_additional(&mut nbt);

        assert_eq!(
            nbt.string("LootTable").map(ToString::to_string),
            Some("minecraft:chests/abandoned_mineshaft".to_owned())
        );
        assert_eq!(nbt.long("LootTableSeed"), Some(42));
        assert_eq!(nbt.byte("HasTicked"), Some(1));
        assert_eq!(nbt.byte("FlippedRotation"), Some(0));
    }

    #[test]
    fn chest_minecart_is_pickable_and_pushable_like_vanilla() {
        let minecart = ChestMinecartEntity::new(
            &vanilla_entities::CHEST_MINECART,
            1,
            DVec3::new(1.5, 2.5, 3.5),
            Weak::new(),
        );

        assert!(minecart.is_pickable());
        assert!(minecart.is_pushable());
        assert!(minecart.blocks_building());
    }

    #[test]
    fn chest_minecart_relative_portal_position_resets_forward_offset() {
        let minecart = ChestMinecartEntity::new(
            &vanilla_entities::CHEST_MINECART,
            1,
            DVec3::new(12.0, 66.0, 20.75),
            Weak::new(),
        );
        let portal_area = FoundRectangle {
            min_corner: steel_utils::BlockPos::new(10, 64, 20),
            axis1_size: 4,
            axis2_size: 5,
        };

        assert!(
            minecart
                .get_relative_portal_position(Axis::X, portal_area)
                .z
                .abs()
                < f64::EPSILON
        );
    }
}
