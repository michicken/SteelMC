//! Minimal Litematica (`.litematic`) projection reader / writer plus placement,
//! used by machine acceptance tests that must start from a real projection
//! format instead of hand-placed blocks.
//!
//! The format (see Litematica's `LitematicaSchematic`):
//! ```text
//! root
//!   Regions: { <name>: {
//!     Position: {x,y,z},
//!     Size: {x,y,z},
//!     BlockStatePalette: [ {Name, Properties}, ... ],
//!     BlockStates: [long, ...],   // LSB-first packed palette indices,
//!                                 // x fastest, then z, then y
//!     TileEntities: [...],
//!   } }
//! ```
//! `.litematic` files are gzip-compressed NBT. The reader also tolerates
//! uncompressed NBT so fixtures can be written without gzip if desired.

use std::fs;
use std::io::{Cursor, Read, Write};
use std::path::Path;
use std::str::FromStr;
use std::sync::Arc;

use flate2::Compression;
use flate2::read::GzDecoder;
use flate2::write::GzEncoder;
use simdnbt::borrow::read as read_nbt;
use simdnbt::owned::{Nbt, NbtCompound, NbtList, NbtTag};
use steel_registry::REGISTRY;
use steel_registry::blocks::block_state_ext::BlockStateExt as _;
use steel_registry::shared_structs::BlockStateData;
use steel_utils::nbt::NbtValueInput as _;
use steel_utils::{BlockPos, BlockStateId, Identifier, types::UpdateFlags};
use steel_worldgen::state_resolver::WorldgenStateResolver;

use crate::world::World;

/// A tile-entity payload carried by the projection (chest contents, sign text, ...).
#[derive(Debug, Clone)]
pub(crate) struct LitematicTileEntity {
    /// Region-local block position (`x,y,z`).
    pub(crate) pos: [i32; 3],
    pub(crate) nbt: NbtCompound,
}

/// One region of a litematic projection, decoded into registry block states.
#[derive(Debug)]
pub(crate) struct LitematicRegion {
    pub(crate) name: String,
    /// Region origin relative to the schematic origin (Litematica `Position`).
    pub(crate) position: [i32; 3],
    pub(crate) size: [i32; 3],
    pub(crate) palette: Vec<BlockStateId>,
    /// Palette index per block, indexed by `x + z * size_x + y * size_x * size_z`.
    indices: Vec<usize>,
    /// Block-entity payloads saved alongside the blocks.
    pub(crate) tile_entities: Vec<LitematicTileEntity>,
}

impl LitematicRegion {
    #[must_use]
    pub(crate) fn state_at(&self, x: i32, y: i32, z: i32) -> BlockStateId {
        let index = self.linear_index(x, y, z);
        self.palette[self.indices[index]]
    }

    #[must_use]
    pub(crate) fn linear_index(&self, x: i32, y: i32, z: i32) -> usize {
        debug_assert!(x >= 0 && y >= 0 && z >= 0);
        debug_assert!(x < self.size[0] && y < self.size[1] && z < self.size[2]);
        x as usize
            + z as usize * self.size[0] as usize
            + y as usize * self.size[0] as usize * self.size[2] as usize
    }

    #[must_use]
    pub(crate) fn volume(&self) -> usize {
        self.size[0] as usize * self.size[1] as usize * self.size[2] as usize
    }

    /// Number of non-air blocks in this region.
    #[must_use]
    pub(crate) fn solid_block_count(&self) -> usize {
        let air = REGISTRY.blocks.state_id_from_properties(
            &Identifier::from_str("minecraft:air").expect("air identifier"),
            &[],
        );
        self.indices
            .iter()
            .filter(|&&index| Some(self.palette[index]) != air)
            .count()
    }
}

/// A decoded litematic projection.
#[derive(Debug)]
pub(crate) struct Litematic {
    pub(crate) name: String,
    pub(crate) regions: Vec<LitematicRegion>,
}

impl Litematic {
    /// Total number of non-air blocks across all regions.
    #[must_use]
    pub(crate) fn solid_block_count(&self) -> usize {
        self.regions.iter().map(LitematicRegion::solid_block_count).sum()
    }

    /// All distinct block states used by the projection.
    #[must_use]
    pub(crate) fn all_states(&self) -> Vec<BlockStateId> {
        let mut states: Vec<BlockStateId> = self
            .regions
            .iter()
            .flat_map(|region| region.palette.iter().copied())
            .collect();
        states.sort_by_key(|state| state.0);
        states.dedup();
        states
    }
}

/// Read a `.litematic` file (gzip NBT, with a plain-NBT fallback).
pub(crate) fn load_litematic(path: &Path) -> Result<Litematic, String> {
    let bytes = fs::read(path).map_err(|err| format!("read {}: {err}", path.display()))?;
    let mut decompressed = Vec::new();
    match GzDecoder::new(bytes.as_slice()).read_to_end(&mut decompressed) {
        Ok(_) => {}
        Err(_) => decompressed = bytes,
    }
    parse_litematic(&decompressed)
}

/// Parse litematic NBT bytes (already decompressed).
pub(crate) fn parse_litematic(data: &[u8]) -> Result<Litematic, String> {
    let nbt = read_nbt(&mut Cursor::new(data)).map_err(|err| format!("parse nbt: {err}"))?;
    let root = match nbt {
        simdnbt::borrow::Nbt::Some(root) => root,
        simdnbt::borrow::Nbt::None => return Err("empty nbt".to_owned()),
    };
    let root = root.as_compound();
    let regions_nbt = root
        .compound("Regions")
        .ok_or_else(|| "litematic missing Regions compound".to_owned())?;

    let name = root
        .compound("Metadata")
        .and_then(|metadata| metadata.string("Name"))
        .map_or_else(|| "unnamed".to_owned(), |name| name.to_str().into_owned());

    let mut regions = Vec::new();
    for (region_name, region_tag) in regions_nbt.iter() {
        let region_name = region_name.to_str().into_owned();
        let Some(region) = region_tag.compound() else {
            return Err(format!("region {region_name} is not a compound"));
        };
        regions.push(parse_region(&region, region_name)?);
    }
    if regions.is_empty() {
        return Err("litematic has no regions".to_owned());
    }
    Ok(Litematic { name, regions })
}

fn read_vec3_i(compound: &simdnbt::borrow::NbtCompound<'_, '_>, key: &str) -> Result<[i32; 3], String> {
    let vec = compound
        .compound(key)
        .ok_or_else(|| format!("missing {key}"))?;
    let x = vec.int_value("x").ok_or_else(|| format!("{key} missing x"))?;
    let y = vec.int_value("y").ok_or_else(|| format!("{key} missing y"))?;
    let z = vec.int_value("z").ok_or_else(|| format!("{key} missing z"))?;
    Ok([x, y, z])
}

fn parse_region(
    region: &simdnbt::borrow::NbtCompound<'_, '_>,
    name: String,
) -> Result<LitematicRegion, String> {
    let position = read_vec3_i(region, "Position")?;
    let raw_size = read_vec3_i(region, "Size")?;
    // Litematica encodes mirrored regions with a negative size; tests only
    // paste axis-aligned structures, so normalise to the absolute extent.
    let size = [
        raw_size[0].abs().max(1),
        raw_size[1].abs().max(1),
        raw_size[2].abs().max(1),
    ];

    let palette_entries = region
        .list("BlockStatePalette")
        .and_then(|list| list.compounds())
        .ok_or_else(|| format!("region {name} missing BlockStatePalette"))?;
    let mut palette = Vec::with_capacity(palette_entries.len());
    for entry in palette_entries.clone() {
        let entry_name = entry
            .string("Name")
            .ok_or_else(|| format!("region {name} palette entry missing Name"))?;
        let identifier = Identifier::from_str(entry_name.to_str().as_ref())
            .map_err(|err| format!("region {name} invalid block id {entry_name}: {err}"))?;
        let mut properties = std::collections::BTreeMap::new();
        if let Some(props) = entry.compound("Properties") {
            for (key, value) in props.iter() {
                let Some(value) = value.string() else {
                    return Err(format!(
                        "region {name} property {} on {identifier} is not a string",
                        key.to_str()
                    ));
                };
                properties.insert(key.to_str().into_owned(), value.to_str().into_owned());
            }
        }
        palette.push(WorldgenStateResolver::block_state_from_data(
            &REGISTRY,
            &BlockStateData {
                name: identifier,
                properties,
            },
            "litematic palette",
        ));
    }
    if palette.is_empty() {
        return Err(format!("region {name} has an empty palette"));
    }

    let volume = size[0] as usize * size[1] as usize * size[2] as usize;
    let longs = region
        .long_array("BlockStates")
        .ok_or_else(|| format!("region {name} missing BlockStates"))?;
    let bits = bits_per_index(palette.len());
    let indices = unpack_indices(&longs, volume, bits);
    if indices.iter().any(|&index| index >= palette.len()) {
        return Err(format!(
            "region {name} has a palette index outside 0..{}",
            palette.len()
        ));
    }

    let mut tile_entities = Vec::new();
    if let Some(entries) = region.list("TileEntities").and_then(|list| list.compounds()) {
        for entry in entries.clone() {
            let pos = entry
                .int_array("pos")
                .ok_or_else(|| format!("region {name} tile entity missing pos int array"))?;
            if pos.len() != 3 {
                return Err(format!("region {name} tile entity pos must have 3 entries"));
            }
            tile_entities.push(LitematicTileEntity {
                pos: [pos[0], pos[1], pos[2]],
                nbt: entry.to_owned(),
            });
        }
    }

    Ok(LitematicRegion {
        name,
        position,
        size,
        palette,
        indices,
        tile_entities,
    })
}

/// Litematica uses at least 2 bits per palette index.
fn bits_per_index(palette_len: usize) -> u32 {
    if palette_len <= 1 {
        return 2;
    }
    let mut bits = 2;
    while (1usize << bits) < palette_len {
        bits += 1;
    }
    bits
}

/// Unpack LSB-first long array into palette indices.
fn unpack_indices(longs: &[i64], count: usize, bits: u32) -> Vec<usize> {
    let mask = (1u64 << bits) - 1;
    let mut result = Vec::with_capacity(count);
    let mut bit_offset = 0usize;
    for _ in 0..count {
        let long_index = bit_offset / 64;
        let in_long = bit_offset % 64;
        let mut value = if long_index < longs.len() {
            (longs[long_index] as u64) >> in_long
        } else {
            0
        };
        if in_long + bits as usize > 64 && long_index + 1 < longs.len() {
            value |= (longs[long_index + 1] as u64) << (64 - in_long);
        }
        result.push((value & mask) as usize);
        bit_offset += bits as usize;
    }
    result
}

/// Pack palette indices into a litematic long array (LSB-first).
fn pack_indices(indices: &[usize], bits: u32) -> Vec<i64> {
    let mask = (1u64 << bits) - 1;
    let total_bits = indices.len() * bits as usize;
    let long_count = total_bits.div_ceil(64);
    let mut longs = vec![0u64; long_count];
    let mut bit_offset = 0usize;
    for &index in indices {
        let value = (index as u64) & mask;
        let long_index = bit_offset / 64;
        let in_long = bit_offset % 64;
        longs[long_index] |= value << in_long;
        if in_long + bits as usize > 64 {
            longs[long_index + 1] |= value >> (64 - in_long);
        }
        bit_offset += bits as usize;
    }
    longs.into_iter().map(|value| value as i64).collect()
}

/// Builds a single-region litematic fixture and writes it to disk.
struct BuilderRegion {
    name: String,
    position: [i32; 3],
    size: [i32; 3],
    palette: Vec<(String, Vec<(String, String)>)>,
    indices: Vec<usize>,
    tile_entities: Vec<([i32; 3], NbtCompound)>,
}

impl BuilderRegion {
    fn new(name: &str, position: [i32; 3], size: (i32, i32, i32)) -> Self {
        Self {
            name: name.to_owned(),
            position,
            size: [size.0, size.1, size.2],
            palette: vec![("minecraft:air".to_owned(), Vec::new())],
            indices: vec![0; size.0 as usize * size.1 as usize * size.2 as usize],
            tile_entities: Vec::new(),
        }
    }

    fn linear_index(&self, x: i32, y: i32, z: i32) -> usize {
        assert!(
            x >= 0 && y >= 0 && z >= 0 && x < self.size[0] && y < self.size[1] && z < self.size[2],
            "litematic region {} position ({x},{y},{z}) outside {:?}",
            self.name,
            self.size
        );
        x as usize
            + z as usize * self.size[0] as usize
            + y as usize * self.size[0] as usize * self.size[2] as usize
    }

    fn palette_index(&mut self, name: &str, properties: &[(&str, &str)]) -> usize {
        let entry = (
            name.to_owned(),
            properties
                .iter()
                .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
                .collect::<Vec<_>>(),
        );
        if let Some(index) = self.palette.iter().position(|existing| *existing == entry) {
            return index;
        }
        self.palette.push(entry);
        self.palette.len() - 1
    }

    fn set(&mut self, x: i32, y: i32, z: i32, name: &str, properties: &[(&str, &str)]) {
        let palette_index = self.palette_index(name, properties);
        let index = self.linear_index(x, y, z);
        self.indices[index] = palette_index;
    }

    fn fill(&mut self, name: &str, properties: &[(&str, &str)]) {
        let palette_index = self.palette_index(name, properties);
        self.indices.fill(palette_index);
    }

    fn write(&self, regions: &mut NbtCompound) {
        let mut region = NbtCompound::new();
        let mut position = NbtCompound::new();
        position.insert("x", self.position[0]);
        position.insert("y", self.position[1]);
        position.insert("z", self.position[2]);
        region.insert("Position", position);
        let mut size = NbtCompound::new();
        size.insert("x", self.size[0]);
        size.insert("y", self.size[1]);
        size.insert("z", self.size[2]);
        region.insert("Size", size);

        let mut palette = Vec::with_capacity(self.palette.len());
        for (name, properties) in &self.palette {
            let mut entry = NbtCompound::new();
            entry.insert("Name", name.clone());
            let mut props = NbtCompound::new();
            for (key, value) in properties {
                props.insert(key.clone(), value.clone());
            }
            entry.insert("Properties", props);
            palette.push(entry);
        }
        region.insert("BlockStatePalette", NbtList::Compound(palette));
        let bits = bits_per_index(self.palette.len());
        region.insert(
            "BlockStates",
            NbtTag::LongArray(pack_indices(&self.indices, bits)),
        );

        let mut tile_entities = Vec::with_capacity(self.tile_entities.len());
        for (pos, nbt) in &self.tile_entities {
            let mut entry = nbt.clone();
            entry.insert("pos", NbtTag::IntArray(pos.to_vec()));
            tile_entities.push(entry);
        }
        region.insert("TileEntities", NbtList::Compound(tile_entities));
        regions.insert(self.name.clone(), region);
    }
}

/// Builds a litematic fixture (one or more regions) and writes it to disk.
pub(crate) struct LitematicBuilder {
    regions: Vec<BuilderRegion>,
}

impl LitematicBuilder {
    /// Creates an all-air single-region structure of `size` blocks.
    #[must_use]
    pub(crate) fn new(name: &str, size: (i32, i32, i32)) -> Self {
        Self {
            regions: vec![BuilderRegion::new(name, [0, 0, 0], size)],
        }
    }

    /// Creates a multi-region structure; each entry is `(name, position, size)`.
    #[must_use]
    pub(crate) fn new_multi(regions: &[(&str, [i32; 3], (i32, i32, i32))]) -> Self {
        assert!(!regions.is_empty(), "a litematic needs at least one region");
        Self {
            regions: regions
                .iter()
                .map(|(name, position, size)| BuilderRegion::new(name, *position, *size))
                .collect(),
        }
    }

    /// Places a block in the first region, registering its palette entry on demand.
    pub(crate) fn set(
        &mut self,
        x: i32,
        y: i32,
        z: i32,
        name: &str,
        properties: &[(&str, &str)],
    ) -> &mut Self {
        self.regions[0].set(x, y, z, name, properties);
        self
    }

    /// Places a block in a specific region.
    pub(crate) fn set_in_region(
        &mut self,
        region: usize,
        x: i32,
        y: i32,
        z: i32,
        name: &str,
        properties: &[(&str, &str)],
    ) -> &mut Self {
        self.regions[region].set(x, y, z, name, properties);
        self
    }

    /// Fills the whole first region with one block.
    pub(crate) fn fill(&mut self, name: &str, properties: &[(&str, &str)]) -> &mut Self {
        self.regions[0].fill(name, properties);
        self
    }

    /// Attaches a block-entity payload at a position in the first region.
    pub(crate) fn set_tile_entity(
        &mut self,
        x: i32,
        y: i32,
        z: i32,
        nbt: NbtCompound,
    ) -> &mut Self {
        let index = self.regions[0].linear_index(x, y, z);
        let _ = index;
        self.regions[0].tile_entities.push(([x, y, z], nbt));
        self
    }

    /// Attaches a block-entity payload at a position in a specific region.
    pub(crate) fn set_tile_entity_in_region(
        &mut self,
        region: usize,
        x: i32,
        y: i32,
        z: i32,
        nbt: NbtCompound,
    ) -> &mut Self {
        let index = self.regions[region].linear_index(x, y, z);
        let _ = index;
        self.regions[region].tile_entities.push(([x, y, z], nbt));
        self
    }

    /// Serialises the fixture as gzip-compressed litematic NBT.
    pub(crate) fn write(&self, path: &Path) -> Result<(), String> {
        let mut root = NbtCompound::new();
        root.insert("Version", 6i32);
        root.insert("MinecraftDataVersion", 3953i32);
        let mut metadata = NbtCompound::new();
        metadata.insert("Name", self.regions[0].name.clone());
        root.insert("Metadata", metadata);

        let mut regions = NbtCompound::new();
        for region in &self.regions {
            region.write(&mut regions);
        }
        root.insert("Regions", regions);

        let mut nbt_bytes = Vec::new();
        Nbt::new("".into(), root).write(&mut nbt_bytes);
        let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
        encoder
            .write_all(&nbt_bytes)
            .map_err(|err| format!("gzip write: {err}"))?;
        let compressed = encoder.finish().map_err(|err| format!("gzip finish: {err}"))?;
        fs::write(path, compressed).map_err(|err| format!("write {}: {err}", path.display()))
    }
}

/// Result of pasting a litematic into a world.
#[derive(Debug, Default)]
pub(crate) struct PasteReport {
    pub(crate) placed: usize,
    pub(crate) skipped_air: usize,
    pub(crate) failed_positions: Vec<BlockPos>,
    pub(crate) tile_entities_loaded: usize,
    pub(crate) failed_tile_entities: Vec<BlockPos>,
}

/// Paste every region of `litematic` into `world` at `origin`.
///
/// Blocks are written with `UPDATE_CLIENTS` only so a partially assembled
/// machine never fires mid-paste, then a second pass replays vanilla's
/// structure-placement neighbour notifications for every placed block.
/// Block entities are created by `set_block`.
pub(crate) fn paste_litematic(
    world: &Arc<World>,
    litematic: &Litematic,
    origin: BlockPos,
) -> PasteReport {
    let air = REGISTRY
        .blocks
        .state_id_from_properties(&Identifier::from_str("minecraft:air").expect("air"), &[])
        .expect("air state");
    let mut report = PasteReport::default();
    let mut placed_positions = Vec::new();
    for region in &litematic.regions {
        for y in 0..region.size[1] {
            for z in 0..region.size[2] {
                for x in 0..region.size[0] {
                    let state = region.state_at(x, y, z);
                    if state == air {
                        report.skipped_air += 1;
                        continue;
                    }
                    let pos = origin.offset(
                        region.position[0] + x,
                        region.position[1] + y,
                        region.position[2] + z,
                    );
                    if world.set_block(pos, state, UpdateFlags::UPDATE_CLIENTS) {
                        report.placed += 1;
                        placed_positions.push((pos, state.get_block()));
                    } else {
                        report.failed_positions.push(pos);
                    }
                }
            }
        }
    }
    for (pos, block) in placed_positions {
        world.update_neighbors_at(pos, block);
    }

    // Tile entities are loaded after the block + neighbour passes so containers
    // exist and a half-assembled machine never reads their contents.
    for region in &litematic.regions {
        for tile_entity in &region.tile_entities {
            let pos = origin.offset(
                region.position[0] + tile_entity.pos[0],
                region.position[1] + tile_entity.pos[1],
                region.position[2] + tile_entity.pos[2],
            );
            let Some(block_entity) = world.get_block_entity(pos) else {
                report.failed_tile_entities.push(pos);
                continue;
            };
            if block_entity.load_with_owned_components(&tile_entity.nbt).is_err() {
                report.failed_tile_entities.push(pos);
            } else {
                report.tile_entities_loaded += 1;
            }
        }
    }
    report
}
