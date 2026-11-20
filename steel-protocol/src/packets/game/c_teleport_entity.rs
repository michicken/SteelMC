use glam::DVec3;
use steel_macros::{ClientPacket, WriteTo};
use steel_registry::packets::play::C_TELEPORT_ENTITY;

use super::c_player_position::RelativeMovement;

/// Teleports an entity with optional relative position, rotation, and velocity flags.
#[derive(ClientPacket, WriteTo, Clone, Debug)]
#[packet_id(Play = C_TELEPORT_ENTITY)]
pub struct CTeleportEntity {
    #[write(as = VarInt)]
    pub entity_id: i32,
    pub pos: DVec3,
    pub vel: DVec3,
    pub yaw: f32,
    pub pitch: f32,
    pub relatives: RelativeMovement,
    pub on_ground: bool,
}

impl CTeleportEntity {
    #[must_use]
    pub const fn new(
        entity_id: i32,
        pos: DVec3,
        vel: DVec3,
        yaw: f32,
        pitch: f32,
        relatives: RelativeMovement,
        on_ground: bool,
    ) -> Self {
        Self {
            entity_id,
            pos,
            vel,
            yaw,
            pitch,
            relatives,
            on_ground,
        }
    }
}

#[cfg(test)]
mod tests {
    use glam::DVec3;
    use steel_utils::serial::WriteTo;

    use super::{CTeleportEntity, RelativeMovement};

    /// Locks the current wire layout so an accidental encoder change is caught.
    ///
    /// Layout: entity id (`VarInt`), position (`f64` x3), velocity (`f64` x3),
    /// yaw (`f32`), pitch (`f32`), relative flags (`i32`), on-ground (`bool`).
    /// The velocity stays double-precision here instead of the `LpVec3`
    /// compression used by `CAddEntity`/`CSetEntityMotion` because teleports
    /// must not quantise motion.
    #[test]
    fn teleport_entity_wire_layout_is_stable() {
        let packet = CTeleportEntity::new(
            42,
            DVec3::new(1.0, 2.0, 3.0),
            DVec3::new(0.0, 0.0, 0.0),
            0.0,
            0.0,
            RelativeMovement::NONE,
            false,
        );

        let mut bytes = Vec::new();
        WriteTo::write(&packet, &mut bytes).expect("encoding must not fail");

        // VarInt entity id (42) + 3 x f64 position + 3 x f64 velocity
        // + f32 yaw + f32 pitch + i32 flags + bool on-ground.
        assert_eq!(bytes.len(), 1 + 24 + 24 + 4 + 4 + 4 + 1);
        assert_eq!(bytes[0], 0x2A);
        assert_eq!(&bytes[1..9], &1.0_f64.to_be_bytes());
        assert_eq!(&bytes[9..17], &2.0_f64.to_be_bytes());
        assert_eq!(&bytes[17..25], &3.0_f64.to_be_bytes());
        // Velocity is still plain doubles, so it must not be a packed short.
        assert_eq!(&bytes[25..49], &[0u8; 24]);
        // yaw, pitch and flags are all zero for this case.
        assert_eq!(&bytes[49..61], &[0u8; 12]);
        assert_eq!(bytes[61], 0x00);
    }

    #[test]
    fn teleport_entity_preserves_relative_flags_and_motion() {
        let packet = CTeleportEntity::new(
            42,
            DVec3::new(1.0, 2.0, 3.0),
            DVec3::new(0.1, 0.2, 0.3),
            90.0,
            -15.0,
            RelativeMovement::DELTA.union(RelativeMovement::ROTATION),
            true,
        );

        assert_eq!(packet.entity_id, 42);
        assert_eq!(packet.pos, DVec3::new(1.0, 2.0, 3.0));
        assert_eq!(packet.vel, DVec3::new(0.1, 0.2, 0.3));
        assert_eq!(packet.yaw, 90.0);
        assert_eq!(packet.pitch, -15.0);
        assert_eq!(
            packet.relatives,
            RelativeMovement::DELTA.union(RelativeMovement::ROTATION)
        );
        assert!(packet.on_ground);
    }
}
