use std::io::{Result, Write};

use steel_macros::ClientPacket;
use steel_registry::packets::play::C_BOSS_EVENT;
use steel_utils::codec::VarInt;
use steel_utils::serial::WriteTo;
use text_components::TextComponent;
use uuid::Uuid;

/// Boss bar operation, matching vanilla's `ClientboundBossEventPacket.Operation`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BossEventOperation {
    /// Create the bar with name, progress, colour, overlay and flags.
    Add,
    /// Remove the bar.
    Remove,
    /// Update the health fraction.
    UpdateProgress,
    /// Update the displayed name.
    UpdateName,
    /// Update colour and overlay style.
    UpdateStyle,
    /// Update the player-friendly flags.
    UpdateProperties,
}

impl BossEventOperation {
    const fn id(self) -> i32 {
        match self {
            Self::Add => 0,
            Self::Remove => 1,
            Self::UpdateProgress => 2,
            Self::UpdateName => 3,
            Self::UpdateStyle => 4,
            Self::UpdateProperties => 5,
        }
    }
}

/// Boss event packet: creates, updates and removes a boss bar.
#[derive(ClientPacket, Clone, Debug)]
#[packet_id(Play = C_BOSS_EVENT)]
pub struct CBossEvent {
    /// Identity shared by every packet for one boss bar.
    pub uuid: Uuid,
    /// Which operation this packet performs.
    pub operation: BossEventOperation,
    /// Displayed name for `Add` and `UpdateName`.
    pub name: Option<TextComponent>,
    /// Health fraction in `0.0..=1.0` for `Add` and `UpdateProgress`.
    pub progress: f32,
    /// Vanilla boss-bar colour id for `Add` and `UpdateStyle`.
    pub color: i32,
    /// Vanilla overlay id (0 progress, 1 notched) for `Add` and `UpdateStyle`.
    pub overlay: i32,
    /// Vanilla flags byte for `Add` and `UpdateProperties`.
    pub flags: u8,
}

impl CBossEvent {
    /// Creates an `Add` packet.
    ///
    /// `overlay` is the vanilla notched-bar id (0 plain, 1–6 notched); `flags` is the
    /// bitfield of darken-sky / dragon-bar / boss-music / world-fog display flags.
    #[must_use]
    pub fn add(
        uuid: Uuid,
        name: impl Into<TextComponent>,
        progress: f32,
        color: i32,
        overlay: i32,
        flags: u8,
    ) -> Self {
        Self {
            uuid,
            operation: BossEventOperation::Add,
            name: Some(name.into()),
            progress,
            color,
            overlay,
            flags,
        }
    }

    /// Creates a `Remove` packet.
    #[must_use]
    pub const fn remove(uuid: Uuid) -> Self {
        Self {
            uuid,
            operation: BossEventOperation::Remove,
            name: None,
            progress: 0.0,
            color: 0,
            overlay: 0,
            flags: 0,
        }
    }

    /// Creates an `UpdateProgress` packet.
    #[must_use]
    pub const fn update_progress(uuid: Uuid, progress: f32) -> Self {
        Self {
            uuid,
            operation: BossEventOperation::UpdateProgress,
            name: None,
            progress,
            color: 0,
            overlay: 0,
            flags: 0,
        }
    }
}

impl WriteTo for CBossEvent {
    fn write(&self, writer: &mut impl Write) -> Result<()> {
        self.uuid.write(writer)?;
        VarInt(self.operation.id()).write(writer)?;
        match self.operation {
            BossEventOperation::Add => {
                if let Some(name) = &self.name {
                    name.write(writer)?;
                }
                self.progress.write(writer)?;
                VarInt(self.color).write(writer)?;
                VarInt(self.overlay).write(writer)?;
                self.flags.write(writer)?;
            }
            BossEventOperation::Remove => {}
            BossEventOperation::UpdateProgress => self.progress.write(writer)?,
            BossEventOperation::UpdateName => {
                if let Some(name) = &self.name {
                    name.write(writer)?;
                }
            }
            BossEventOperation::UpdateStyle => {
                VarInt(self.color).write(writer)?;
                VarInt(self.overlay).write(writer)?;
            }
            BossEventOperation::UpdateProperties => self.flags.write(writer)?,
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn add_carries_color_overlay_and_flags() {
        let uuid = Uuid::nil();
        let packet = CBossEvent::add(uuid, "Dragon", 0.75, 4, 1, 0x2);

        assert_eq!(packet.operation, BossEventOperation::Add);
        assert_eq!(packet.color, 4);
        assert_eq!(packet.overlay, 1);
        assert_eq!(packet.flags, 0x2);
        assert_eq!(packet.progress, 0.75);
        assert!(packet.name.is_some());
    }

    #[test]
    fn remove_zeroes_the_display_fields() {
        let packet = CBossEvent::remove(Uuid::nil());

        assert_eq!(packet.operation, BossEventOperation::Remove);
        assert!(packet.name.is_none());
        assert_eq!(packet.progress, 0.0);
        assert_eq!(packet.color, 0);
        assert_eq!(packet.overlay, 0);
        assert_eq!(packet.flags, 0);
    }
}
