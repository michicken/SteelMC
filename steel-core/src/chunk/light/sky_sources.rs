use steel_registry::{blocks::block_state_ext::BlockStateExt, vanilla_blocks};
use steel_utils::{BlockStateId, Direction, SectionPos};

use crate::chunk::section::Sections;

use super::{
    CHUNK_COLUMN_COUNT, CHUNK_EDGE, LightSectionRangeError, NEGATIVE_INFINITY, light_face_occludes,
};

/// Per-chunk cache of the lowest skylight source edge in each X/Z column.
///
/// Uninitialized chunks and all-air chunks share the implicit lower source edge.
/// Allocate per-column values only when a column has a higher source edge.
#[derive(Debug, Clone, Eq)]
pub struct ChunkSkyLightSources {
    min_y: i32,
    heightmap: Option<Box<[i32; CHUNK_COLUMN_COUNT]>>,
}

impl PartialEq for ChunkSkyLightSources {
    fn eq(&self, other: &Self) -> bool {
        self.min_y == other.min_y
            && match (&self.heightmap, &other.heightmap) {
                (None, None) => true,
                (Some(a), Some(b)) => a == b,
                (Some(columns), None) | (None, Some(columns)) => {
                    columns.iter().all(|&height| height == self.min_y)
                }
            }
    }
}

impl ChunkSkyLightSources {
    /// Creates an empty skylight-source cache for a level height.
    pub const fn new(min_y: i32, height: i32) -> Result<Self, LightSectionRangeError> {
        if height <= 0 || min_y.checked_sub(1).is_none() || min_y.checked_add(height).is_none() {
            return Err(LightSectionRangeError { min_y, height });
        }

        let min_y = min_y - 1;
        Ok(Self {
            min_y,
            heightmap: None,
        })
    }

    /// Creates a cache for world heights already accepted by chunk construction.
    ///
    /// Invalid world heights are fatal because chunks and light sections cannot
    /// be indexed coherently without a valid vertical range.
    ///
    /// # Panics
    ///
    /// Panics when the supplied world height cannot form a valid light-section range.
    #[must_use]
    pub fn for_valid_world_height(min_y: i32, height: i32) -> Self {
        Self::new(min_y, height).expect("invalid world height for skylight sources")
    }

    /// Fills this cache from a chunk's sections.
    pub fn fill_from_sections(&mut self, sections: &Sections) {
        let Some(sections) = sections.materialized() else {
            self.heightmap = None;
            return;
        };
        let air = Self::air_state();
        let mut upper_states = [air; CHUNK_COLUMN_COUNT];
        let mut sources = [self.min_y; CHUNK_COLUMN_COUNT];
        let mut remaining = CHUNK_COLUMN_COUNT;

        // Walk every column through the same descending edges while holding one
        // section read lock. Empty sections reset the upper face to air.
        for (section_index, holder) in sections.iter().enumerate().rev() {
            let section = holder.read();
            if section.is_empty() {
                upper_states.fill(air);
                continue;
            }
            let section_min_y =
                Self::section_to_block_coord(self.section_y_from_index(section_index));
            let states = section.states();
            for (column, source) in sources.iter_mut().enumerate() {
                if *source != self.min_y {
                    continue;
                }
                let x = column % CHUNK_EDGE;
                let z = column / CHUNK_EDGE;
                let upper_state = &mut upper_states[column];
                for y in (0..CHUNK_EDGE).rev() {
                    let lower_state = states.get(x, y, z);
                    if Self::is_edge_occluded(*upper_state, lower_state) {
                        *source = section_min_y + y as i32 + 1;
                        remaining -= 1;
                        break;
                    }
                    *upper_state = lower_state;
                }
            }
            if remaining == 0 {
                break;
            }
        }
        self.heightmap = (remaining < CHUNK_COLUMN_COUNT).then(|| Box::new(sources));
    }

    /// Updates one column after a block change.
    ///
    /// `state_at` is called with section-local X/Z and world Y coordinates.
    /// Returns true when the cached source edge changed.
    pub fn update(
        &mut self,
        x: usize,
        y: i32,
        z: usize,
        mut state_at: impl FnMut(usize, i32, usize) -> BlockStateId,
    ) -> bool {
        debug_assert!(x < CHUNK_EDGE);
        debug_assert!(z < CHUNK_EDGE);

        let Some(upper_edge_y) = y.checked_add(1) else {
            return false;
        };
        let index = Self::index(x, z);
        let current_lowest_source_y = self.get(index);
        if upper_edge_y < current_lowest_source_y {
            return false;
        }

        let top_state = state_at(x, upper_edge_y, z);
        let middle_state = state_at(x, y, z);
        if self.update_edge(
            index,
            current_lowest_source_y,
            x,
            z,
            upper_edge_y,
            top_state,
            y,
            middle_state,
            &mut state_at,
        ) {
            return true;
        }

        let Some(bottom_y) = y.checked_sub(1) else {
            return false;
        };
        let bottom_state = state_at(x, bottom_y, z);
        self.update_edge(
            index,
            current_lowest_source_y,
            x,
            z,
            y,
            middle_state,
            bottom_y,
            bottom_state,
            &mut state_at,
        )
    }

    /// Returns the lowest skylight source Y for a local X/Z column.
    #[must_use]
    pub fn get_lowest_source_y(&self, x: usize, z: usize) -> i32 {
        self.extend_sources_below_world(self.get(Self::index(x, z)))
    }

    /// Returns the highest cached lowest-source Y across all columns.
    #[must_use]
    pub fn get_highest_lowest_source_y(&self) -> i32 {
        let max_value = self.heightmap.as_deref().map_or(self.min_y, |columns| {
            columns.iter().copied().max().unwrap_or(self.min_y)
        });
        self.extend_sources_below_world(max_value)
    }

    #[expect(
        clippy::too_many_arguments,
        reason = "mirrors vanilla's updateEdge inputs without bundling temporary positions"
    )]
    fn update_edge(
        &mut self,
        index: usize,
        old_top_edge_y: i32,
        x: usize,
        z: usize,
        checked_edge_y: i32,
        top_state: BlockStateId,
        bottom_y: i32,
        bottom_state: BlockStateId,
        state_at: &mut impl FnMut(usize, i32, usize) -> BlockStateId,
    ) -> bool {
        if Self::is_edge_occluded(top_state, bottom_state) {
            if checked_edge_y > old_top_edge_y {
                self.set(index, checked_edge_y);
                return true;
            }
        } else if checked_edge_y == old_top_edge_y {
            let new_source_y =
                self.find_lowest_source_below(x, z, bottom_y, bottom_state, state_at);
            self.set(index, new_source_y);
            return true;
        }

        false
    }

    fn find_lowest_source_below(
        &self,
        x: usize,
        z: usize,
        start_y: i32,
        start_state: BlockStateId,
        state_at: &mut impl FnMut(usize, i32, usize) -> BlockStateId,
    ) -> i32 {
        let mut top_y = start_y;
        let mut top_state = start_state;
        let Some(mut bottom_y) = start_y.checked_sub(1) else {
            return self.min_y;
        };

        while bottom_y >= self.min_y {
            let bottom_state = state_at(x, bottom_y, z);
            if Self::is_edge_occluded(top_state, bottom_state) {
                return top_y;
            }

            top_state = bottom_state;
            top_y = bottom_y;
            let Some(next_bottom_y) = bottom_y.checked_sub(1) else {
                break;
            };
            bottom_y = next_bottom_y;
        }

        self.min_y
    }

    fn is_edge_occluded(top_state: BlockStateId, bottom_state: BlockStateId) -> bool {
        if bottom_state.get_light_dampening() != 0 {
            return true;
        }

        light_face_occludes(top_state, bottom_state, Direction::Down)
    }

    fn set(&mut self, index: usize, value: i32) {
        if self.heightmap.is_none() && value == self.min_y {
            return;
        }
        self.heightmap
            .get_or_insert_with(|| Box::new([self.min_y; CHUNK_COLUMN_COUNT]))[index] = value;
    }

    fn get(&self, index: usize) -> i32 {
        self.heightmap
            .as_deref()
            .map_or(self.min_y, |columns| columns[index])
    }

    const fn extend_sources_below_world(&self, value: i32) -> i32 {
        if value == self.min_y {
            NEGATIVE_INFINITY
        } else {
            value
        }
    }

    const fn section_y_from_index(&self, section_index: usize) -> i32 {
        SectionPos::block_to_section_coord(self.min_y + 1) + section_index as i32
    }

    const fn section_to_block_coord(section_y: i32) -> i32 {
        section_y << 4
    }

    const fn index(x: usize, z: usize) -> usize {
        x + z * CHUNK_EDGE
    }

    fn air_state() -> BlockStateId {
        vanilla_blocks::AIR.default_state()
    }
}
