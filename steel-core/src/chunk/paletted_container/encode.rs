//! Network encoding of paletted container entries.
//!
//! Entries are packed like vanilla `SimpleBitStorage`: `64 / bits` entries per long, low
//! bits first, no entry spanning two longs, longs written big-endian.

use std::io::{Error, ErrorKind, Result};

use super::ToGlobalId;

/// Largest packed payload: a 16×16×16 section at 16 bits per entry.
pub(super) const MAX_PACKED_BYTES: usize = super::BlockPalette::VOLUME.div_ceil(64 / 16) * 8;
/// Packs each value's global id into `buffer` and returns the packed bytes.
///
/// # Errors
/// Returns [`ErrorKind::InvalidData`] if a global id does not fit in `bits`.
#[cfg(test)]
pub(super) fn pack_global_ids<'buffer, V: ToGlobalId + Copy>(
    values: &[V],
    bits: u8,
    buffer: &'buffer mut [u8; MAX_PACKED_BYTES],
) -> Result<&'buffer [u8]> {
    // Checked once up front so the packing loop has no per-entry branch.
    let all_ids = values
        .iter()
        .fold(0, |all_ids, value| all_ids | value.to_global_id());
    if all_ids >> u32::from(bits) != 0 {
        return Err(invalid_data(
            "container holds a global id wider than the global palette",
        ));
    }
    pack(values, bits, |value| Some(value.to_global_id()), buffer)
}

/// Packs `values` into `buffer`, failing if a value has no entry.
#[cfg(test)]
fn pack<'buffer, V: Copy>(
    values: &[V],
    bits: u8,
    lookup: impl FnMut(V) -> Option<u32>,
    buffer: &'buffer mut [u8; MAX_PACKED_BYTES],
) -> Result<&'buffer [u8]> {
    // A width known at compile time lets each long be filled by a fixed-length loop.
    macro_rules! pack_with_width {
        ($($width:literal)*) => {
            match bits {
                $($width => pack_longs::<V, $width>(values, lookup, buffer),)*
                _ => Err(invalid_data("unsupported bits per entry")),
            }
        };
    }
    pack_with_width!(1 2 3 4 5 6 7 8 9 10 11 12 13 14 15 16)
}

/// Packs `values` into the front of `buffer`, failing if a value has no entry.
#[cfg(test)]
fn pack_longs<'buffer, V: Copy, const BITS: usize>(
    values: &[V],
    mut lookup: impl FnMut(V) -> Option<u32>,
    buffer: &'buffer mut [u8; MAX_PACKED_BYTES],
) -> Result<&'buffer [u8]> {
    let values_per_long = 64 / BITS;
    let len = values.len().div_ceil(values_per_long) * 8;
    let packed = buffer
        .get_mut(..len)
        .ok_or_else(|| invalid_data("container is too large to pack"))?;
    // Accumulate misses instead of branching per entry; the whole container is rejected.
    let mut missing = false;
    for (entries, long_bytes) in values
        .chunks(values_per_long)
        .zip(packed.as_chunks_mut::<8>().0)
    {
        let mut long = 0u64;
        for (slot, value) in entries.iter().enumerate() {
            let id = lookup(*value);
            missing |= id.is_none();
            long |= u64::from(id.unwrap_or(0)) << (slot * BITS);
        }
        *long_bytes = long.to_be_bytes();
    }
    if missing {
        return Err(invalid_data(
            "container holds a value its palette cannot encode",
        ));
    }
    Ok(packed)
}

/// Packs raw palette indices, rejecting an index at or past `palette_len`.
pub(super) fn pack_local_indices<'buffer>(
    count: usize,
    palette_len: usize,
    bits: u8,
    indices: impl Iterator<Item = u32>,
    buffer: &'buffer mut [u8; MAX_PACKED_BYTES],
) -> Result<&'buffer [u8]> {
    let mut indices = indices;
    pack_entries(
        count,
        bits,
        move |_| {
            let index = indices.next()?;
            (usize::try_from(index).ok()? < palette_len).then_some(index)
        },
        buffer,
    )
}

/// Packs global ids looked up from raw palette indices.
pub(super) fn pack_global_indices<'buffer, V: ToGlobalId + Copy>(
    count: usize,
    palette: &[(V, u16)],
    bits: u8,
    indices: impl Iterator<Item = u32>,
    buffer: &'buffer mut [u8; MAX_PACKED_BYTES],
) -> Result<&'buffer [u8]> {
    let mut all_ids = 0u32;
    for (value, _) in palette {
        all_ids |= value.to_global_id();
    }
    if all_ids >> u32::from(bits) != 0 {
        return Err(invalid_data(
            "container holds a global id wider than the global palette",
        ));
    }
    let mut indices = indices;
    pack_entries(
        count,
        bits,
        move |_| {
            let index = usize::try_from(indices.next()?).ok()?;
            palette.get(index).map(|(value, _)| value.to_global_id())
        },
        buffer,
    )
}

/// Packs one entry per index, low bits first and never crossing a long.
fn pack_entries<'buffer>(
    count: usize,
    bits: u8,
    mut lookup: impl FnMut(usize) -> Option<u32>,
    buffer: &'buffer mut [u8; MAX_PACKED_BYTES],
) -> Result<&'buffer [u8]> {
    macro_rules! pack_with_width {
        ($($width:literal)*) => {
            match bits {
                $($width => pack_entries_width::<$width>(count, &mut lookup, buffer),)*
                _ => Err(invalid_data("unsupported bits per entry")),
            }
        };
    }
    pack_with_width!(1 2 3 4 5 6 7 8 9 10 11 12 13 14 15 16)
}

fn pack_entries_width<'buffer, const BITS: usize>(
    count: usize,
    lookup: &mut impl FnMut(usize) -> Option<u32>,
    buffer: &'buffer mut [u8; MAX_PACKED_BYTES],
) -> Result<&'buffer [u8]> {
    let values_per_long = 64 / BITS;
    let len = count.div_ceil(values_per_long) * 8;
    let packed = buffer
        .get_mut(..len)
        .ok_or_else(|| invalid_data("container is too large to pack"))?;
    let mut missing = false;
    for (word_index, long_bytes) in packed.as_chunks_mut::<8>().0.iter_mut().enumerate() {
        let mut long = 0u64;
        for slot in 0..values_per_long {
            let index = word_index * values_per_long + slot;
            if index >= count {
                break;
            }
            let id = lookup(index);
            missing |= id.is_none();
            long |= u64::from(id.unwrap_or(0)) << (slot * BITS);
        }
        *long_bytes = long.to_be_bytes();
    }
    if missing {
        return Err(invalid_data(
            "container holds a value its palette cannot encode",
        ));
    }
    Ok(packed)
}

fn invalid_data(message: &'static str) -> Error {
    Error::new(ErrorKind::InvalidData, message)
}

#[cfg(test)]
mod tests {
    use super::{MAX_PACKED_BYTES, pack_global_ids};

    /// Packs values at every width from 1 to 16 bits, including 9 to 15, which no section
    /// uses with vanilla's registries, and reads them back the way vanilla's
    /// `SimpleBitStorage` does: `64 / bits` values per big-endian long, low bits first.
    #[test]
    fn packs_every_width_like_simple_bit_storage() {
        let mut state = 0x9E37_79B9_7F4A_7C15_u64;
        for bits in 1..=16u8 {
            let mask = (1u64 << bits) - 1;
            let values: Vec<u16> = (0..4096)
                .map(|_| {
                    state ^= state << 13;
                    state ^= state >> 7;
                    state ^= state << 17;
                    (state & mask) as u16
                })
                .collect();
            let mut buffer = [0; MAX_PACKED_BYTES];
            let packed =
                pack_global_ids(&values, bits, &mut buffer).expect("every value fits the width");

            let values_per_long = 64 / usize::from(bits);
            let (longs, rest) = packed.as_chunks::<8>();
            assert!(
                rest.is_empty(),
                "{bits} bits: packed bytes should be whole longs"
            );
            assert_eq!(
                longs.len(),
                values.len().div_ceil(values_per_long),
                "{bits} bits"
            );
            for (index, &value) in values.iter().enumerate() {
                let long = u64::from_be_bytes(longs[index / values_per_long]);
                let entry = (long >> (index % values_per_long * usize::from(bits))) & mask;
                assert_eq!(entry, u64::from(value), "{bits} bits, value {index}");
            }
        }
    }
}
