//! A paletted container is a container that can be either homogeneous or heterogeneous.
use std::{
    fmt::Debug,
    hash::Hash,
    io::{Result, Write},
    mem, slice,
    sync::OnceLock,
};

use smallvec::SmallVec;

use steel_registry::blocks::block_state_ext::BlockStateExt;
use steel_registry::{REGISTRY, RegistryExt as _};
use steel_utils::{BlockStateId, codec::VarInt, mth::ceil_log2, serial::WriteTo};

mod encode;
mod words;

use words::PackedWords;

/// A trait for converting a value to a global ID.
pub trait ToGlobalId {
    /// Converts the value to a global ID.
    fn to_global_id(&self) -> u32;
}

impl ToGlobalId for BlockStateId {
    fn to_global_id(&self) -> u32 {
        u32::from(self.0)
    }
}

impl ToGlobalId for u16 {
    fn to_global_id(&self) -> u32 {
        u32::from(*self)
    }
}

/// 3d array indexed by y,z,x
type Cube<T, const DIM: usize> = [[[T; DIM]; DIM]; DIM];

/// Packed palette indices, `bits` bits per entry, low bits first in each `u64`.
///
/// Widths use vanilla's 1/2/3/4/5/6/7/8/16 layout. A word holds
/// `64 / bits` entries; the few leftover high bits (for 5, 6 and 7 bits) stay
/// unused, which keeps every entry inside one word.
#[derive(Debug, Clone)]
struct PackedIndices {
    bits: u8,
    values_per_word: u8,
    /// Reciprocal for `index / values_per_word`; every hot path avoids `div`.
    word_mul: u32,
    data: PackedWords,
}

impl PackedIndices {
    fn new(bits: u8, volume: usize) -> Self {
        debug_assert!(matches!(bits, 1 | 2 | 3 | 4 | 5 | 6 | 7 | 8 | 16));
        let values_per_word = 64 / usize::from(bits);
        debug_assert!(values_per_word * usize::from(bits) <= 64);
        Self {
            bits,
            values_per_word: values_per_word as u8,
            word_mul: ((1_u64 << 32) / values_per_word as u64 + 1) as u32,
            data: PackedWords::Dense(vec![0; volume.div_ceil(values_per_word)].into_boxed_slice()),
        }
    }

    #[inline]
    fn word_index(&self, index: usize) -> (usize, usize) {
        let word = (u64::from(self.word_mul) * index as u64) >> 32;
        let word = word as usize;
        (word, index - word * usize::from(self.values_per_word))
    }

    #[inline]
    fn get(&self, index: usize, volume: usize) -> u32 {
        debug_assert!(index < volume);
        let bits = usize::from(self.bits);
        let (word_index, offset) = self.word_index(index);
        let word = self.data.get(word_index);
        let shift = offset * bits;
        ((word >> shift) & ((1u64 << bits) - 1)) as u32
    }

    /// Fills a contiguous run with one index, one bit-field write per touched word.
    ///
    /// Worldgen cubes are highly run-length friendly (16-block layers of stone or
    /// air), so this turns most of `from_cube` into a few full-word stores instead
    /// of 4096 read-modify-write operations.
    fn fill(&mut self, start: usize, len: usize, value: u32, volume: usize) {
        if len == 0 {
            return;
        }
        debug_assert!(start + len <= volume);
        let bits = usize::from(self.bits);
        let values_per_word = usize::from(self.values_per_word);
        let unit_mask = (1_u64 << bits) - 1;
        let word_mul = self.word_mul;
        let data = self.data.dense_mut();
        let mut index = start;
        let end = start + len;
        while index < end {
            let word_index = ((u64::from(word_mul) * index as u64) >> 32) as usize;
            let offset = index - word_index * values_per_word;
            let take = (values_per_word - offset).min(end - index);
            let total_bits = take * bits;
            let low_mask = if total_bits == 64 {
                u64::MAX
            } else {
                (1_u64 << total_bits) - 1
            };
            // Repetition count for a value in adjacent bit fields. Division is exact;
            // the product below fills every field without carries because each field
            // is `bits` wide and `value` fits in `bits`.
            let repeat = if total_bits == 64 {
                u64::MAX / unit_mask
            } else {
                low_mask / unit_mask
            };
            let shift = offset * bits;
            let mask = low_mask << shift;
            let word = &mut data[word_index];
            *word = (*word & !mask) | ((u64::from(value).wrapping_mul(repeat) << shift) & mask);
            index += take;
        }
    }

    #[inline]
    fn set(&mut self, index: usize, value: u32, volume: usize) {
        debug_assert!(index < volume);
        let bits = usize::from(self.bits);
        let (word_index, offset) = self.word_index(index);
        let shift = offset * bits;
        let mask = ((1u64 << bits) - 1) << shift;
        let word = &mut self.data.dense_mut()[word_index];
        *word = (*word & !mask) | (u64::from(value) << shift);
    }

    /// Replaces one existing palette index in every packed word.
    fn replace_index(&mut self, from: u32, to: u32) {
        let bits = u32::from(self.bits);
        let entry_mask = (1_u64 << bits) - 1;
        debug_assert!(from != 0 && u64::from(from | to) <= entry_mask);

        let used_bits = bits * u32::from(self.values_per_word);
        let used_mask = u64::MAX >> (u64::BITS - used_bits);
        let lows = used_mask / entry_mask;
        let highs = lows << (bits - 1);
        let lower_bits = highs - lows;
        let target = u64::from(from) * lows;
        let delta = u64::from(from ^ to);

        for word in self.data.dense_mut() {
            // In each entry, adding lower_bits sets its high bit if any lower
            // bit differs. The addition cannot carry into the next entry.
            let differences = *word ^ target;
            let nonmatching = ((differences & !highs) + lower_bits) | differences;
            let matching_low_bits = (highs & !nonmatching) >> (bits - 1);
            *word ^= matching_low_bits * delta;
        }
    }

    fn iter(&self, volume: usize) -> PackedIndicesIter<'_> {
        PackedIndicesIter {
            words: &self.data,
            next_word: 0,
            bits: self.bits,
            values_per_word: self.values_per_word,
            mask: (1_u64 << self.bits) - 1,
            word: 0,
            entries_in_word: 0,
            remaining: volume,
        }
    }

    fn words(&self, volume: usize) -> impl ExactSizeIterator<Item = u64> + '_ {
        (0..volume.div_ceil(usize::from(self.values_per_word))).map(|index| self.data.get(index))
    }

    /// Rewrites the entries at a wider or narrower width.
    fn repack(&mut self, new_bits: u8, volume: usize) {
        if self.bits == new_bits {
            return;
        }
        let old = mem::replace(self, Self::new(new_bits, volume));
        for (index, value) in old.iter(volume).enumerate() {
            self.set(index, value, volume);
        }
    }
}

struct PackedIndicesIter<'a> {
    words: &'a PackedWords,
    next_word: usize,
    bits: u8,
    values_per_word: u8,
    mask: u64,
    word: u64,
    entries_in_word: u8,
    remaining: usize,
}

impl Iterator for PackedIndicesIter<'_> {
    type Item = u32;

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        if self.remaining == 0 {
            return None;
        }
        if self.entries_in_word == 0 {
            self.word = self.words.get(self.next_word);
            self.next_word += 1;
            self.entries_in_word = self.values_per_word;
        }

        let value = (self.word & self.mask) as u32;
        self.word >>= self.bits;
        self.entries_in_word -= 1;
        self.remaining -= 1;
        Some(value)
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        (self.remaining, Some(self.remaining))
    }
}

impl ExactSizeIterator for PackedIndicesIter<'_> {}

/// A heterogeneous palette container.
///
/// Every value in the cube appears exactly once in the palette. Cells store a
/// packed palette index instead of the full value, cutting section memory from
/// 8 KiB to 0.5–4 KiB for typical terrain palettes.
#[derive(Debug, Clone)]
pub struct HeterogeneousPalette<V: Hash + Eq + Copy, const DIM: usize> {
    // Keeps track of how many different times each value appears in the cube. (value, count)
    palette: Vec<(V, u16)>,
    indices: PackedIndices,
}

impl<V: Hash + Eq + Copy, const DIM: usize> HeterogeneousPalette<V, DIM> {
    const VOLUME: usize = DIM * DIM * DIM;

    /// Runtime storage is independent of the wire format: the serializers repack
    /// to vanilla's 4/5/6/7/8-bit layout, so small palettes can use 1 or 2 bits.
    #[inline]
    fn bits_for_palette_len(len: usize) -> u8 {
        match len {
            0 | 1 | 2 => 1,
            3..=4 => 2,
            5..=8 => 3,
            9..=16 => 4,
            17..=32 => 5,
            33..=64 => 6,
            65..=128 => 7,
            129..=256 => 8,
            _ => 16,
        }
    }

    /// The distinct values in the cube, each with the number of cells holding it.
    pub(crate) fn palette(&self) -> &[(V, u16)] {
        &self.palette
    }

    /// Packed palette indices in y, z, x order.
    pub(crate) fn iter_indices(&self) -> impl ExactSizeIterator<Item = u32> + '_ {
        self.indices.iter(Self::VOLUME)
    }

    /// Returns existing packed words when the persistent layout matches.
    pub(crate) fn words_for_bits(
        &self,
        bits: u8,
    ) -> Option<impl ExactSizeIterator<Item = u64> + '_> {
        (self.indices.bits == bits).then(|| self.indices.words(Self::VOLUME))
    }

    /// All values in y, z, x order.
    pub(crate) fn iter_values(&self) -> impl Iterator<Item = V> + '_ {
        self.indices.iter(Self::VOLUME).map(|index| {
            self.palette
                .get(index as usize)
                .unwrap_or_else(|| panic!("palette entry {index} is missing from its palette"))
                .0
        })
    }

    #[inline]
    fn get(&self, x: usize, y: usize, z: usize) -> V {
        debug_assert!(x < DIM);
        debug_assert!(y < DIM);
        debug_assert!(z < DIM);

        let index = x + z * DIM + y * DIM * DIM;
        self.palette[self.indices.get(index, Self::VOLUME) as usize].0
    }

    #[inline]
    fn get_at_index(&self, index: usize) -> V {
        debug_assert!(index < Self::VOLUME);
        self.palette[self.indices.get(index, Self::VOLUME) as usize].0
    }

    /// Builds the palette and packed indices from a raw cube.
    ///
    /// Returns the single value when the cube is homogeneous, avoiding a pointless
    /// packed allocation for the many all-air/all-stone sections in a chunk.
    fn from_cube(cube: &Cube<V, DIM>) -> (V, Option<Self>) {
        let total = Self::VOLUME;
        // SAFETY: `[[[V; DIM]; DIM]; DIM]` is a fully-contiguous array of
        // `DIM*DIM*DIM` `V`s, so casting its base pointer to `*const V` and
        // building a slice of that length is sound.
        let flat: &[V] = unsafe { slice::from_raw_parts(cube.as_ptr().cast::<V>(), total) };

        let mut palette: Vec<(V, u16)> = Vec::new();
        // (start, len, palette index), one entry per contiguous run.
        let mut runs = SmallVec::<[(u32, u16, u32); 64]>::new();
        let mut i = 0;
        while i < total {
            let value = flat[i];
            let mut j = i + 1;
            while j < total && flat[j] == value {
                j += 1;
            }
            let run_len = (j - i) as u16;
            let palette_index = if let Some(pos) = palette.iter().position(|(v, _)| *v == value) {
                palette[pos].1 += run_len;
                pos
            } else {
                palette.push((value, run_len));
                palette.len() - 1
            };
            runs.push((i as u32, run_len, palette_index as u32));
            i = j;
        }

        let first = palette[0].0;
        if palette.len() == 1 {
            return (first, None);
        }

        let bits = Self::bits_for_palette_len(palette.len());
        let mut indices = PackedIndices::new(bits, total);
        for &(start, len, palette_index) in &runs {
            indices.fill(start as usize, usize::from(len), palette_index, total);
        }

        indices.data.compact();
        (first, Some(Self { palette, indices }))
    }

    #[inline]
    fn set(&mut self, x: usize, y: usize, z: usize, value: V) -> V {
        debug_assert!(x < DIM);
        debug_assert!(y < DIM);
        debug_assert!(z < DIM);

        let cell = x + z * DIM + y * DIM * DIM;
        let old_index = self.indices.get(cell, Self::VOLUME) as usize;
        let old_value = self.palette[old_index].0;
        if old_value == value {
            return old_value;
        }

        let new_index = if let Some(pos) = self.palette.iter().position(|(v, _)| *v == value) {
            self.palette[pos].1 += 1;
            pos
        } else {
            self.palette.push((value, 1));
            self.palette.len() - 1
        };

        let old_count = &mut self.palette[old_index].1;
        *old_count -= 1;
        let mut new_index = new_index;
        if *old_count == 0 {
            let last = self.palette.len() - 1;
            if old_index != last {
                let last_was_just_added = new_index == last && self.palette[last].1 == 1;
                self.palette.swap_remove(old_index);
                if new_index == last {
                    new_index = old_index;
                }
                // A count of one at the destination means it was just added
                // for this write and no packed index references it yet.
                if !last_was_just_added {
                    self.indices.replace_index(last as u32, old_index as u32);
                }
            } else {
                self.palette.pop();
            }
        }

        let bits = Self::bits_for_palette_len(self.palette.len());
        if bits != self.indices.bits {
            self.indices.repack(bits, Self::VOLUME);
        }
        self.indices.set(cell, new_index as u32, Self::VOLUME);

        old_value
    }
}

/// A paletted container.
///
/// `Building` is a transient mode used during worldgen: it's a raw cube without
/// palette tracking, so writes are O(1) stores. Must be finalized via
/// [`Self::finalize_building`] (or implicitly when the parent section recalculates
/// its counters) before any serialization or paletted access. Mirrors
/// `FastNoise`'s `FastChunkSection` write-only fill mode.
#[derive(Debug, Clone)]
pub enum PalettedContainer<V: Hash + Eq + Copy + Default, const DIM: usize> {
    /// A homogeneous container, where all values are the same.
    Homogeneous(V),
    /// An allocation for mixed values, keeping homogeneous sections small.
    Heterogeneous(Box<HeterogeneousPalette<V, DIM>>),
    /// Write-only build mode: raw cube without palette tracking.
    /// `set` is a single store; `get` is a direct read.
    /// Convert back via [`Self::finalize_building`].
    Building(Box<Cube<V, DIM>>),
}

enum PaletteMode {
    Linear,
    Hash,
    Global,
}

impl<V: Hash + Eq + Copy + Default + Debug, const DIM: usize> PalettedContainer<V, DIM> {
    /// The size of the container in one dimension.
    pub const SIZE: usize = DIM;
    /// The volume of the container.
    pub const VOLUME: usize = DIM * DIM * DIM;

    /// Creates a `PalettedContainer` from a pre-built cube.
    ///
    /// Will automatically determine if the result should be homogeneous or heterogeneous.
    ///
    /// Walks the cube as a flat slice (it's `[[[V; DIM]; DIM]; DIM]` so memory
    /// is contiguous) and counts identical cells in runs. The inner "find run
    /// end" loop is a vectorizable equality scan, so long stone columns collapse
    /// to a single palette increment.
    #[must_use]
    pub fn from_cube(cube: Box<Cube<V, DIM>>) -> Self {
        let (single, heterogeneous) = HeterogeneousPalette::<V, DIM>::from_cube(&cube);
        heterogeneous.map_or_else(
            || Self::Homogeneous(single),
            |palette| Self::Heterogeneous(Box::new(palette)),
        )
    }

    /// Gets the value at the given coordinates.
    #[inline]
    pub fn get(&self, x: usize, y: usize, z: usize) -> V {
        match self {
            Self::Homogeneous(value) => *value,
            Self::Heterogeneous(data) => data.get(x, y, z),
            Self::Building(cube) => {
                debug_assert!(x < DIM);
                debug_assert!(y < DIM);
                debug_assert!(z < DIM);
                cube[y][z][x]
            }
        }
    }

    /// Gets the value at a y,z,x linear index.
    ///
    /// The index layout is `x + z * DIM + y * DIM * DIM`, matching the flat
    /// order used when serializing palette data and by `ScalableLux` light propagation.
    #[inline]
    pub fn get_at_index(&self, index: usize) -> V {
        debug_assert!(index < Self::VOLUME);

        match self {
            Self::Homogeneous(value) => *value,
            Self::Heterogeneous(data) => data.get_at_index(index),
            Self::Building(cube) => {
                let y = index / (DIM * DIM);
                let z = (index / DIM) % DIM;
                let x = index % DIM;
                cube[y][z][x]
            }
        }
    }

    /// Copies the full vertical column at `(x, z)` into `out`.
    pub(crate) fn copy_column_into(&self, x: usize, z: usize, out: &mut [V]) {
        debug_assert!(x < DIM);
        debug_assert!(z < DIM);
        debug_assert!(out.len() >= DIM);

        match self {
            Self::Homogeneous(value) => {
                for slot in &mut out[..DIM] {
                    *slot = *value;
                }
            }
            Self::Heterogeneous(data) => {
                for (y, slot) in out[..DIM].iter_mut().enumerate() {
                    *slot = data.get(x, y, z);
                }
            }
            Self::Building(cube) => {
                for (y, slot) in out[..DIM].iter_mut().enumerate() {
                    *slot = cube[y][z][x];
                }
            }
        }
    }

    /// Returns whether this container's palette may contain a matching value.
    ///
    /// This checks palette entries instead of every cell, matching vanilla and
    /// `ScalableLux`'s fast pre-scan before doing a full section pass.
    #[must_use]
    pub fn maybe_has(&self, mut predicate: impl FnMut(V) -> bool) -> bool {
        match self {
            Self::Homogeneous(value) => predicate(*value),
            Self::Heterogeneous(data) => data.palette.iter().any(|(value, _)| predicate(*value)),
            Self::Building(cube) => cube
                .iter()
                .flatten()
                .flatten()
                .any(|value| predicate(*value)),
        }
    }

    /// Collects all values in the container in y, z, x order.
    #[must_use]
    pub fn collect_values(&self) -> Vec<V> {
        match self {
            Self::Homogeneous(value) => vec![*value; Self::VOLUME],
            Self::Heterogeneous(data) => data.iter_values().collect(),
            Self::Building(cube) => cube.iter().flatten().flatten().copied().collect(),
        }
    }

    /// Switches the container into write-only build mode for fast bulk writes.
    /// Idempotent: a no-op if already in [`Self::Building`].
    ///
    /// Allocates and fills a `Cube` from the current paletted contents.
    pub fn enter_building_mode(&mut self) {
        match self {
            Self::Building(_) => {}
            Self::Homogeneous(value) => {
                let cube: Box<Cube<V, DIM>> = Box::new([[[*value; DIM]; DIM]; DIM]);
                *self = Self::Building(cube);
            }
            Self::Heterogeneous(_) => {
                let taken = mem::replace(self, Self::Homogeneous(V::default()));
                let Self::Heterogeneous(data) = taken else {
                    unreachable!()
                };
                let mut cube: Box<Cube<V, DIM>> = Box::new([[[V::default(); DIM]; DIM]; DIM]);
                for index in 0..HeterogeneousPalette::<V, DIM>::VOLUME {
                    let value = data.palette[data
                        .indices
                        .get(index, HeterogeneousPalette::<V, DIM>::VOLUME)
                        as usize]
                        .0;
                    let y = index / (DIM * DIM);
                    let z = (index / DIM) % DIM;
                    let x = index % DIM;
                    cube[y][z][x] = value;
                }
                *self = Self::Building(cube);
            }
        }
    }

    /// Returns the raw cube backing this container as a flat mutable slice if
    /// it's currently in [`Self::Building`] mode. Indexing is `[y * DIM*DIM + z * DIM + x]`.
    ///
    /// Used by the chunk fill path to bypass the 3-arm `set` dispatch and the
    /// unused old-value load — write-only worldgen never reads back what it
    /// just wrote, so the read in `set` is wasted memory traffic.
    #[inline]
    pub fn as_building_slice_mut(&mut self) -> Option<&mut [V]> {
        if let Self::Building(cube) = self {
            // SAFETY: `[[[V; DIM]; DIM]; DIM]` is a contiguous array of
            // `DIM*DIM*DIM` `V`s; the cast preserves the live `&mut Box`'s
            // borrow because the returned slice cannot outlive `self`.
            Some(unsafe {
                slice::from_raw_parts_mut(cube.as_mut_ptr().cast::<V>(), DIM * DIM * DIM)
            })
        } else {
            None
        }
    }

    /// Finalizes a [`Self::Building`] container back to `Homogeneous` or
    /// `Heterogeneous` by scanning the cube once and constructing the palette.
    /// No-op if not in build mode.
    pub fn finalize_building(&mut self) {
        if !matches!(self, Self::Building(_)) {
            return;
        }
        let taken = mem::replace(self, Self::Homogeneous(V::default()));
        let Self::Building(cube) = taken else {
            unreachable!()
        };
        *self = Self::from_cube(cube);
    }

    /// Writes `value` without updating the palette, breaking its invariant for tests.
    #[cfg(test)]
    pub(crate) fn set_without_palette_for_test(&mut self, x: usize, y: usize, z: usize, value: V) {
        if let Self::Heterogeneous(data) = self {
            let index = x + z * DIM + y * DIM * DIM;
            // Point at a slot one past the palette: `write` must reject it instead
            // of silently encoding an out-of-range index.
            let _ = value;
            data.indices
                .set(index, data.palette.len() as u32, Self::VOLUME);
        }
    }

    /// Sets the value at the given coordinates.
    #[inline]
    pub fn set(&mut self, x: usize, y: usize, z: usize, value: V) -> V {
        debug_assert!(x < Self::SIZE);
        debug_assert!(y < Self::SIZE);
        debug_assert!(z < Self::SIZE);

        match self {
            Self::Homogeneous(original) => {
                let original = *original;
                if value != original {
                    let mut cube = Box::new([[[original; DIM]; DIM]; DIM]);
                    cube[y][z][x] = value;
                    *self = Self::from_cube(cube);
                }
                original
            }
            Self::Heterogeneous(data) => {
                let original = data.set(x, y, z, value);
                if data.palette.len() == 1 {
                    *self = Self::Homogeneous(data.palette[0].0);
                }
                original
            }
            Self::Building(cube) => {
                let old = cube[y][z][x];
                cube[y][z][x] = value;
                old
            }
        }
    }

    /// Writes the container to the given writer.
    ///
    /// # Errors
    /// - If the writer fails to write.
    /// - If the container holds a value its palette cannot encode: one missing from its
    ///   section palette, or a global id too wide for the global palette. Nothing is
    ///   written then.
    ///
    /// # Panics
    /// - If the container is in [`Self::Building`] mode.
    pub fn write(&self, writer: &mut impl Write) -> Result<()>
    where
        V: ToGlobalId,
    {
        match self {
            Self::Homogeneous(value) => {
                // bits per entry = 0 (ZeroBitStorage)
                0u8.write(writer)?;
                // Single-value palette
                VarInt(value.to_global_id() as i32).write(writer)?;
                // writeFixedSizeLongArray(new long[0]) writes nothing
            }
            Self::Heterogeneous(data) => {
                let (bits, mode) = Self::calculate_strategy(data.palette.len());
                // Packed first, so a broken palette fails before anything is written.
                let mut buffer = [0; encode::MAX_PACKED_BYTES];
                let packed = match mode {
                    PaletteMode::Global => encode::pack_global_indices(
                        Self::VOLUME,
                        &data.palette,
                        bits,
                        data.indices.iter(Self::VOLUME),
                        &mut buffer,
                    )?,
                    PaletteMode::Linear | PaletteMode::Hash if data.indices.bits == bits => {
                        encode::write_local_words(
                            data.indices.words(Self::VOLUME),
                            data.palette.len(),
                            bits,
                            &mut buffer,
                        )?
                    }
                    PaletteMode::Linear | PaletteMode::Hash => encode::pack_local_indices(
                        Self::VOLUME,
                        data.palette.len(),
                        bits,
                        data.indices.iter(Self::VOLUME),
                        &mut buffer,
                    )?,
                };

                // Write bits per entry
                bits.write(writer)?;

                // Write Palette
                match mode {
                    PaletteMode::Linear | PaletteMode::Hash => {
                        VarInt(data.palette.len() as i32).write(writer)?;
                        for (val, _) in &data.palette {
                            VarInt(val.to_global_id() as i32).write(writer)?;
                        }
                    }
                    PaletteMode::Global => {}
                }

                // writeFixedSizeLongArray: raw longs, no VarInt length prefix
                writer.write_all(packed)?;
            }
            Self::Building(_) => {
                panic!(
                    "PalettedContainer in Building mode cannot be serialized; \
                     call finalize_building() first"
                );
            }
        }
        Ok(())
    }

    fn calculate_strategy(count: usize) -> (u8, PaletteMode) {
        if DIM == 16 {
            // Block states
            match count {
                0..=1 => unreachable!("Homogeneous handled separately"),
                2..=16 => (4, PaletteMode::Linear),
                17..=32 => (5, PaletteMode::Hash),
                33..=64 => (6, PaletteMode::Hash),
                65..=128 => (7, PaletteMode::Hash),
                129..=256 => (8, PaletteMode::Hash),
                _ => (block_state_global_bits(), PaletteMode::Global),
            }
        } else {
            // Biomes
            match count {
                0..=1 => unreachable!("Homogeneous handled separately"),
                2 => (1, PaletteMode::Linear),
                3..=4 => (2, PaletteMode::Linear),
                5..=8 => (3, PaletteMode::Hash),
                _ => (biome_global_bits(), PaletteMode::Global),
            }
        }
    }
}

/// Bits per entry for the block-state global palette.
///
/// Derived from the registered block-state count and cached on first use, so a
/// version bump that changes the state count doesn't need a matching hardcoded
/// constant here. Matches vanilla, which sizes the global palette from the
/// registry rather than a fixed width.
fn block_state_global_bits() -> u8 {
    ceil_log2(usize::from(REGISTRY.blocks.next_state_id))
}

/// Bits per entry for the biome global palette. See [`block_state_global_bits`].
fn biome_global_bits() -> u8 {
    static BITS: OnceLock<u8> = OnceLock::new();
    *BITS.get_or_init(|| ceil_log2(REGISTRY.biomes.len()))
}

/// A palette container for blocks.
pub type BlockPalette = PalettedContainer<BlockStateId, 16>;
/// A palette container for biomes.
pub type BiomePalette = PalettedContainer<u16, 4>;

impl BlockPalette {
    /// Gets the number of non-empty blocks in the container.
    #[must_use]
    pub fn non_empty_block_count(&self) -> u16 {
        match self {
            Self::Homogeneous(v) => {
                if v.0 == 0 {
                    0
                } else {
                    #[expect(
                        clippy::cast_possible_truncation,
                        reason = "VOLUME = 16^3 = 4096, fits in u16"
                    )]
                    {
                        Self::VOLUME as u16
                    }
                }
            }
            Self::Heterogeneous(data) => {
                let mut count = 0;
                for (v, c) in &data.palette {
                    if v.0 != 0 {
                        count += c;
                    }
                }
                count
            }
            Self::Building(cube) => {
                let mut count: u16 = 0;
                for slab in cube {
                    for row in slab {
                        for v in row {
                            if v.0 != 0 {
                                count += 1;
                            }
                        }
                    }
                }
                count
            }
        }
    }

    /// Returns `true` if this palette contains only air blocks.
    #[must_use]
    pub fn has_only_air(&self) -> bool {
        match self {
            Self::Homogeneous(v) => v.is_air(),
            //TODO: Use a nonEmpty counter?
            Self::Heterogeneous(_data) => false,
            Self::Building(cube) => cube
                .iter()
                .flatten()
                .flatten()
                .all(steel_utils::BlockStateId::is_air),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::io::{Cursor, ErrorKind};

    use super::{
        BiomePalette, BlockPalette, PackedIndices, PackedWords, ToGlobalId, biome_global_bits,
        block_state_global_bits,
    };
    use steel_registry::init_vanilla_registry;
    use steel_utils::{BlockStateId, codec::VarInt, serial::ReadFrom as _};

    fn assert_column_matches_get(container: &BlockPalette, x: usize, z: usize) {
        let mut column = [BlockStateId::default(); 16];
        container.copy_column_into(x, z, &mut column);
        for (y, state) in column.into_iter().enumerate() {
            assert_eq!(state, container.get(x, y, z));
        }
    }

    #[test]
    fn packed_replacement_keeps_other_indices_and_handles_partial_words() {
        const VOLUME: usize = 131;
        for bits in [1, 2, 3, 4, 5, 6, 7, 8, 16] {
            let mut storage = PackedIndices::new(bits, VOLUME);
            let from = (1_u32 << bits) - 1;
            let to = from / 2;
            let expected = (0..VOLUME)
                .map(|index| {
                    let value = match index % 3 {
                        0 => from,
                        1 => 0,
                        _ => (index as u32).wrapping_mul(17) & from,
                    };
                    storage.set(index, value, VOLUME);
                    if value == from { to } else { value }
                })
                .collect::<Vec<_>>();

            storage.replace_index(from, to);

            assert_eq!(storage.iter(VOLUME).collect::<Vec<_>>(), expected);
            for (index, value) in expected.into_iter().enumerate() {
                assert_eq!(storage.get(index, VOLUME), value);
            }
        }
    }

    #[test]
    fn packed_vertical_end_runs_round_trip_and_allow_mutations() {
        const VOLUME: usize = BlockPalette::VOLUME;
        let mut indices = PackedIndices::new(5, VOLUME);
        let mut expected = vec![0; VOLUME];
        for (index, entry) in expected.iter_mut().enumerate() {
            *entry = match index {
                ..1300 => 3,
                1300..2700 => (index % 23) as u32,
                _ => 2,
            };
            indices.set(index, *entry, VOLUME);
        }

        indices.data.compact();
        assert!(matches!(indices.data, PackedWords::Trimmed(_)));
        assert!(indices.iter(VOLUME).eq(expected.iter().copied()));
        for (index, &value) in expected.iter().enumerate() {
            assert_eq!(indices.get(index, VOLUME), value);
        }

        indices.set(0, 6, VOLUME);
        indices.set(VOLUME - 1, 8, VOLUME);
        indices.repack(7, VOLUME);
        expected[0] = 6;
        expected[VOLUME - 1] = 8;
        assert!(indices.iter(VOLUME).eq(expected));
    }

    #[test]
    fn get_at_index_reads_homogeneous_palette_values() {
        let container = BlockPalette::Homogeneous(BlockStateId(42));

        assert_eq!(container.get_at_index(0), BlockStateId(42));
        assert_eq!(
            container.get_at_index(BlockPalette::VOLUME - 1),
            BlockStateId(42)
        );
    }

    #[test]
    fn get_at_index_uses_y_z_x_linear_order_for_blocks() {
        let mut container = BlockPalette::Homogeneous(BlockStateId(0));

        container.set(3, 4, 5, BlockStateId(99));

        assert_eq!(
            container.get_at_index(3 + 5 * 16 + 4 * 16 * 16),
            BlockStateId(99)
        );
    }

    #[test]
    fn maybe_has_checks_palette_values_without_scanning_cells() {
        let mut container = BlockPalette::Homogeneous(BlockStateId(0));
        assert!(!container.maybe_has(|state| state == BlockStateId(7)));

        container.set(1, 2, 3, BlockStateId(7));
        assert!(container.maybe_has(|state| state == BlockStateId(7)));
        assert!(!container.maybe_has(|state| state == BlockStateId(9)));
    }

    #[test]
    fn copy_column_into_matches_get_for_homogeneous_container() {
        let container = BlockPalette::Homogeneous(BlockStateId(7));
        assert_column_matches_get(&container, 3, 12);
    }

    #[test]
    fn copy_column_into_matches_get_for_heterogeneous_container() {
        let x = 5;
        let z = 9;
        let mut cube = Box::new([[[BlockStateId::default(); 16]; 16]; 16]);
        for y in 0..16 {
            cube[y][z][x] = BlockStateId((y + 1) as u16);
        }

        let container = BlockPalette::from_cube(cube);
        assert_column_matches_get(&container, x, z);
    }

    #[test]
    fn copy_column_into_matches_get_for_building_container() {
        let x = 11;
        let z = 2;
        let mut container = BlockPalette::Homogeneous(BlockStateId(3));
        container.enter_building_mode();
        for y in 0..16 {
            container.set(x, y, z, BlockStateId((31 + y) as u16));
        }

        assert_column_matches_get(&container, x, z);
    }

    /// The written long array has no length prefix, so a wrong bit width
    /// desyncs every section parsed after this one.
    #[test]
    fn block_global_palette_uses_registry_derived_bit_width_and_long_count() {
        init_vanilla_registry();
        let mut cube = Box::new([[[BlockStateId::default(); 16]; 16]; 16]);
        let mut next_id = 1u16;
        for slab in &mut cube {
            for row in slab.iter_mut() {
                for value in row.iter_mut() {
                    *value = BlockStateId(next_id);
                    next_id = next_id.wrapping_add(1);
                }
            }
        }
        let container = BlockPalette::from_cube(cube);

        let mut written = Vec::new();
        container
            .write(&mut written)
            .expect("writing a global-palette container should not fail");

        let expected_bits = block_state_global_bits();
        assert_eq!(written[0], expected_bits);
        let values_per_long = 64 / usize::from(expected_bits);
        let expected_longs = BlockPalette::VOLUME.div_ceil(values_per_long);
        assert_eq!(written.len(), 1 + expected_longs * 8);
    }

    #[test]
    fn biome_global_palette_uses_registry_derived_bit_width_and_long_count() {
        init_vanilla_registry();
        let mut cube = Box::new([[[0u16; 4]; 4]; 4]);
        let mut next_id = 1u16;
        for slab in &mut cube {
            for row in slab.iter_mut() {
                for value in row.iter_mut() {
                    *value = next_id;
                    next_id += 1;
                }
            }
        }
        let container = BiomePalette::from_cube(cube);

        let mut written = Vec::new();
        container
            .write(&mut written)
            .expect("writing a global-palette container should not fail");

        let expected_bits = biome_global_bits();
        assert_eq!(written[0], expected_bits);
        let values_per_long = 64 / usize::from(expected_bits);
        let expected_longs = BiomePalette::VOLUME.div_ceil(values_per_long);
        assert_eq!(written.len(), 1 + expected_longs * 8);
    }

    /// Reads `write` output the way the vanilla client does: a bits byte, the palette
    /// (absent for the global palette), then `64 / bits` entries per big-endian long.
    /// Returns each entry's global id.
    fn decode_written(bytes: &[u8], volume: usize, max_paletted_bits: u8) -> Vec<u32> {
        let bits = bytes[0];
        let mut cursor = Cursor::new(bytes);
        cursor.set_position(1);
        if bits == 0 {
            let value = VarInt::read(&mut cursor).expect("single value").0 as u32;
            assert_eq!(cursor.position() as usize, bytes.len());
            return vec![value; volume];
        }
        let palette = (bits <= max_paletted_bits).then(|| {
            let len = VarInt::read(&mut cursor).expect("palette length").0 as u32;
            (0..len)
                .map(|_| VarInt::read(&mut cursor).expect("palette entry").0 as u32)
                .collect::<Vec<_>>()
        });
        let bits = usize::from(bits);
        let values_per_long = 64 / bits;
        let longs: Vec<u64> = bytes[cursor.position() as usize..]
            .as_chunks::<8>()
            .0
            .iter()
            .map(|long| u64::from_be_bytes(*long))
            .collect();
        assert_eq!(longs.len(), volume.div_ceil(values_per_long));
        (0..volume)
            .map(|index| {
                let long = longs[index / values_per_long];
                let entry = (long >> ((index % values_per_long) * bits)) & ((1 << bits) - 1);
                match &palette {
                    Some(palette) => palette[entry as usize],
                    None => entry as u32,
                }
            })
            .collect()
    }

    fn xorshift(state: &mut u64) -> u64 {
        *state ^= *state << 13;
        *state ^= *state >> 7;
        *state ^= *state << 17;
        *state
    }

    /// Covers each block palette strategy, including the widths where a long holds a
    /// partial final group (5, 6 and 7 bits).
    #[test]
    fn written_block_sections_decode_to_their_values_for_every_strategy() {
        init_vanilla_registry();
        let mut rng = 0x9E37_79B9_7F4A_7C15_u64;
        for distinct in [2u16, 16, 17, 33, 65, 129, 256, 257] {
            let mut cube = Box::new([[[BlockStateId::default(); 16]; 16]; 16]);
            for (index, value) in cube.iter_mut().flatten().flatten().enumerate() {
                // Every state appears at least once, the rest are random.
                let state = if index < usize::from(distinct) {
                    index as u16
                } else {
                    (xorshift(&mut rng) % u64::from(distinct)) as u16
                };
                *value = BlockStateId(1 + state);
            }
            let container = BlockPalette::from_cube(cube);
            let mut written = Vec::new();
            container
                .write(&mut written)
                .expect("a valid container should encode");

            let expected: Vec<u32> = container
                .collect_values()
                .iter()
                .map(ToGlobalId::to_global_id)
                .collect();
            assert_eq!(
                decode_written(&written, BlockPalette::VOLUME, 8),
                expected,
                "{distinct} distinct states"
            );
        }
    }

    #[test]
    fn random_sets_match_a_naive_model_across_palette_growth_and_shrink() {
        let mut rng = 0x1234_5678_9ABC_DEF0_u64;
        let mut container = BlockPalette::Homogeneous(BlockStateId(0));
        let mut model = [BlockStateId(0); BlockPalette::VOLUME];
        for _ in 0..4000 {
            let x = (xorshift(&mut rng) as usize) % 16;
            let y = (xorshift(&mut rng) as usize) % 16;
            let z = (xorshift(&mut rng) as usize) % 16;
            let state = BlockStateId((xorshift(&mut rng) % 24) as u16);
            let old = container.set(x, y, z, state);
            let index = x + z * 16 + y * 16 * 16;
            assert_eq!(old, model[index]);
            model[index] = state;
        }
        for x in 0..16 {
            for y in 0..16 {
                for z in 0..16 {
                    assert_eq!(container.get(x, y, z), model[x + z * 16 + y * 16 * 16]);
                }
            }
        }
        assert_eq!(container.collect_values(), model);
    }

    #[test]
    fn written_biome_sections_decode_to_their_values_for_every_strategy() {
        init_vanilla_registry();
        for distinct in [2u16, 3, 5, 9] {
            let mut cube = Box::new([[[0u16; 4]; 4]; 4]);
            for (index, value) in cube.iter_mut().flatten().flatten().enumerate() {
                *value = index as u16 % distinct;
            }
            let container = BiomePalette::from_cube(cube);
            let mut written = Vec::new();
            container
                .write(&mut written)
                .expect("a valid container should encode");

            let expected: Vec<u32> = container
                .collect_values()
                .iter()
                .map(|v| u32::from(*v))
                .collect();
            assert_eq!(
                decode_written(&written, BiomePalette::VOLUME, 3),
                expected,
                "{distinct} distinct biomes"
            );
        }
    }

    /// Global-palette ids are written without a section palette, so an id wider than the
    /// global width would spill into the neighboring entry instead of being rejected.
    #[test]
    fn write_rejects_global_id_too_wide_for_the_global_palette() {
        init_vanilla_registry();
        let global_bits = biome_global_bits();
        let mut cube = Box::new([[[0u16; 4]; 4]; 4]);
        for (index, value) in cube.iter_mut().flatten().flatten().enumerate() {
            *value = index as u16;
        }
        cube[0][0][0] = 1 << global_bits;
        let container = BiomePalette::from_cube(cube);

        let mut written = Vec::new();
        let error = container
            .write(&mut written)
            .expect_err("an id too wide for the global palette must not encode");
        assert_eq!(error.kind(), ErrorKind::InvalidData);
        assert!(written.is_empty(), "nothing should be written on failure");
    }

    /// The encoder reuses a per-thread index table. A section holding a value missing from
    /// its palette must be rejected even when an earlier section indexed that value.
    #[test]
    fn write_rejects_value_missing_from_palette_after_it_was_indexed_elsewhere() {
        let stray = BlockStateId(7);
        let mut primer_cube = Box::new([[[BlockStateId(1); 16]; 16]; 16]);
        primer_cube[0][0][1] = stray;
        let mut written = Vec::new();
        BlockPalette::from_cube(primer_cube)
            .write(&mut written)
            .expect("a valid container should encode");

        let mut cube = Box::new([[[BlockStateId(1); 16]; 16]; 16]);
        cube[0][0][1] = BlockStateId(2);
        cube[0][0][2] = BlockStateId(3);
        let mut corrupt = BlockPalette::from_cube(cube);
        corrupt.set_without_palette_for_test(0, 0, 0, stray);

        let mut written = Vec::new();
        let error = corrupt
            .write(&mut written)
            .expect_err("a value missing from the palette must not encode");
        assert_eq!(error.kind(), ErrorKind::InvalidData);
        assert!(written.is_empty(), "nothing should be written on failure");
    }
}
