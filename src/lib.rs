//! Reads Windows PRI files (`resources.pri`, format `mrm_pri2`) into a typed model.
//!
//! The model keeps everything a writer needs to reproduce a file byte for byte: raw padding,
//! tails, name blocks and the header fields we don't interpret. Lengths, offsets and the
//! sections' footers are not kept; they follow from the rest. The format is described in
//! docs/pri-plan.md, section 2.

mod crc;
mod listing;
mod read;

use std::fmt;

pub use read::read;

/// What went wrong while reading, and where.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Error {
    /// Byte offset in the file.
    pub offset: usize,
    pub expected: String,
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "at offset {:#x}: expected {}",
            self.offset, self.expected
        )
    }
}

impl std::error::Error for Error {}

/// A whole file: the header's fields we keep, and the sections in file order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pri {
    pub magic: String,
    pub header_u8: u16,
    pub header_u10: u16,
    pub header_u26: u16,
    pub header_u28: u32,
    pub sections: Vec<Section>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Section {
    /// The 16-byte identifier, e.g. `[mrm_hschemaex] `.
    pub id: [u8; 16],
    pub toc_flags: u16,
    pub toc_section_flags: u16,
    pub toc_qualifier: u32,
    pub header_qualifier: u32,
    pub header_flags: u16,
    pub header_section_flags: u16,
    pub header_u28: u32,
    pub body: Body,
}

impl Section {
    /// The identifier without its padding, e.g. `[mrm_hschemaex]`.
    pub fn id_text(&self) -> String {
        id_text(&self.id)
    }
}

/// A 16-byte identifier without its NUL and space padding.
pub fn id_text(id: &[u8; 16]) -> String {
    String::from_utf8_lossy(id)
        .trim_matches(['\0', ' '])
        .to_string()
}

/// A section's body, parsed when we know the identifier, raw otherwise (padding included).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Body {
    Descriptor(Descriptor),
    /// `None` is the empty schema, a body of no bytes at all.
    Schema(Option<Schema>),
    Decisions(DecnInfo),
    Map(ResMap),
    Data(DataItem),
    Raw(Vec<u8>),
}

/// `[mrm_pridescex]`: the section indices of each kind.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Descriptor {
    pub flags: u16,
    pub included_file_list: u16,
    pub u4: u16,
    pub primary_map: u16,
    pub u18: u16,
    pub schemas: Vec<u16>,
    pub decn_infos: Vec<u16>,
    pub res_maps: Vec<u16>,
    pub file_lists: Vec<u16>,
    pub data_items: Vec<u16>,
    pub tail: Vec<u8>,
}

/// `[mrm_hschema]` and `[mrm_hschemaex] `: the resource names.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Schema {
    /// Mirrors the section id: `[mrm_hschemaex] ` rather than `[mrm_hschema]`.
    pub extended: bool,
    /// The extended form's names-block identifier, e.g. `[def_hnamesx]  \0`.
    pub hnames_id: Option<[u8; 16]>,
    pub major: u16,
    pub minor: u16,
    pub checksum: u32,
    pub unique_name: String,
    pub name: String,
    pub max_full_path: u16,
    /// The names block's own padded total: padding the writer must not add twice.
    pub total: u32,
    pub nodes: Vec<Node>,
    pub scopes: Vec<Scope>,
    /// Node indices of the items, by item index.
    pub items: Vec<u16>,
    /// The UTF-16 name block, raw.
    pub names_uni: Vec<u8>,
    /// The ASCII name block, raw (only `[def_hnamesx]` has one).
    pub names_ascii: Vec<u8>,
    /// Whatever follows the names block in the section's body: padding the writer must not
    /// add twice. The same goes for every other `pad` and `tail`.
    pub pad: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Node {
    /// Parent's node index (the root is its own parent).
    pub parent: u16,
    pub full_len: u16,
    /// The name's uppercased first character, 0 for none.
    pub first: u16,
    pub name_len: u8,
    pub is_scope: bool,
    pub is_ascii: bool,
    /// The flags byte's bits 0xC0, kept as read.
    pub flags_hi: u8,
    /// Offset into the name block, with the flags' low nibble as bits 16 to 19.
    pub offset: u32,
    /// The scope or item index.
    pub index: u16,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Scope {
    pub node: u16,
    pub children: u16,
    pub first_child: u16,
}

impl Schema {
    /// The node's path from the root, `/`-separated (`Files/Assets/X.png`).
    pub fn full_name(&self, node: usize) -> String {
        let mut parts = Vec::new();
        let mut i = node;
        // `read` rejects parent cycles; the limit only guards a hand-built model.
        for _ in 0..self.nodes.len() {
            let Some(n) = self.nodes.get(i) else { break };
            if n.parent as usize == i || n.parent == 0xFFFF {
                break;
            }
            parts.push(n.name.as_str());
            i = n.parent as usize;
        }
        parts.reverse();
        parts.join("/")
    }

    /// MRT Core's `ComputeHierarchicalSchemaVersionChecksum` (mrmmin/HSchema.cpp).
    pub fn checksum(&self) -> u32 {
        let mut crc = hash_str(0, &self.unique_name);
        crc = hash_str(crc, &self.name);
        crc = hash_u32(crc, self.major as u32 | (self.minor as u32) << 16);
        let scopes: Vec<u16> = self.scopes.iter().map(|s| s.node).collect();
        for pool in [&scopes, &self.items] {
            crc = crc::crc32(crc, &[0; 8]); // the pool's description: none
            crc = hash_u32(crc, 1); // case-insensitive
            crc = hash_u32(crc, pool.len() as u32);
            for &node in pool.iter() {
                crc = hash_str(crc, &self.full_name(node as usize));
            }
        }
        crc
    }
}

fn hash_u32(crc: u32, v: u32) -> u32 {
    crc::crc32(crc, &v.to_le_bytes())
}

/// A string as MRT hashes it: its byte length, then its lowercased UTF-16 with a terminator.
fn hash_str(crc: u32, text: &str) -> u32 {
    let data: Vec<u8> = text
        .to_ascii_lowercase()
        .encode_utf16()
        .chain([0])
        .flat_map(u16::to_le_bytes)
        .collect();
    let crc = hash_u32(crc, data.len() as u32);
    crc::crc32(crc, &data)
}

/// The qualifier types, by their number in `[mrm_decn_info]`.
pub const QUALIFIER_TYPES: [&str; 12] = [
    "Language",
    "Contrast",
    "Scale",
    "HomeRegion",
    "TargetSize",
    "LayoutDirection",
    "Theme",
    "AlternateForm",
    "DXFeatureLevel",
    "Configuration",
    "DeviceFamily",
    "Custom",
];

/// `[mrm_decn_info]`: qualifiers, qualifier sets and decisions.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecnInfo {
    /// (first index-table entry, count of qualifier sets).
    pub decisions: Vec<(u16, u16)>,
    /// (first index-table entry, count of qualifiers).
    pub sets: Vec<(u16, u16)>,
    pub quals: Vec<Qualifier>,
    pub distinct: Vec<Distinct>,
    pub index_table: Vec<u16>,
    /// The UTF-16 value block, raw.
    pub values: Vec<u8>,
    pub pad: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Qualifier {
    pub distinct: u16,
    pub priority: u16,
    pub fallback: u16,
    pub reserved: u16,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Distinct {
    pub env: u16,
    pub qualifier_type: u16,
    pub op: u16,
    pub value_type: u16,
    /// Offset into the value block, in UTF-16 units.
    pub value_offset: u32,
    pub value: String,
}

impl Distinct {
    /// The type's name, or its number when we don't know it.
    pub fn type_name(&self) -> String {
        match QUALIFIER_TYPES.get(self.qualifier_type as usize) {
            Some(name) => (*name).to_string(),
            None => self.qualifier_type.to_string(),
        }
    }
}

/// `[mrm_res_map__]` and `[mrm_res_map2_]`: items to candidates.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResMap {
    /// Mirrors the section id: `[mrm_res_map2_]` rather than `[mrm_res_map__]`.
    pub v2: bool,
    pub env: Vec<u8>,
    pub env_count: u16,
    pub schema_section: u16,
    pub schema_ref: Vec<u8>,
    pub decn_section: u16,
    pub types: Vec<(u32, u32)>,
    /// (first item, group index).
    pub item_to_group: Vec<(u16, u16)>,
    /// (item count, first info).
    pub groups: Vec<(u16, u16)>,
    /// Per item: (decision, first candidate).
    pub infos: Vec<(u16, u16)>,
    pub large: Vec<u8>,
    pub candidates: Vec<Candidate>,
    pub data: Vec<u8>,
    pub pad: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Candidate {
    /// A value in a data item section.
    Ref {
        value_type: u8,
        reserved: u16,
        /// Index among that section's values.
        item: u16,
        /// The data item's section index.
        section: u16,
    },
    Embedded {
        value_type: u8,
        reserved: u16,
        value: u32,
    },
}

/// `[mrm_dataitem]`: the values candidates point at.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DataItem {
    pub u0: u32,
    /// (offset, length) into `data`.
    pub strs: Vec<(u16, u16)>,
    pub blobs: Vec<(u32, u32)>,
    pub data: Vec<u8>,
    pub pad: Vec<u8>,
}

impl DataItem {
    /// The strings, then the blobs, as byte runs (ranges are checked when reading).
    pub fn values(&self) -> Vec<&[u8]> {
        let strs = self.strs.iter().map(|&(o, n)| (o as usize, n as usize));
        let blobs = self.blobs.iter().map(|&(o, n)| (o as usize, n as usize));
        strs.chain(blobs)
            .map(|(o, n)| self.data.get(o..o + n).unwrap_or(&[]))
            .collect()
    }
}

impl Pri {
    /// Each resource map's items with their candidates, as `pri dump` prints them.
    pub fn listing(&self) -> Result<String, Error> {
        listing::listing(self)
    }
}
