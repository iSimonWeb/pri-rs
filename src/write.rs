//! Model to bytes. Every length, offset, count and checksum is computed from the model, never
//! copied; the schema's name blocks are rebuilt from the node names, and a data item's values
//! are laid out afresh. Padding the model keeps (`pad`, `tail`, raw bodies) is written as it is.

use std::fmt;

use crate::{Body, Candidate, DataItem, DecnInfo, Descriptor, Pri, ResMap, Schema, Section};

const SECTION_FOOTER: u32 = 0xDEF5_FADE;
const FILE_FOOTER: u32 = 0xDEFF_FADE;
const HEADER_LEN: usize = 32;
const TOC_ENTRY_LEN: usize = 32;
const SECTION_HEADER_LEN: usize = 32;
const FILE_FOOTER_LEN: usize = 16;
const HNAMESX_ID: &[u8; 16] = b"[def_hnamesx]  \0";

/// A model that can't be written: a count, name or offset that doesn't fit its field.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WriteError {
    pub what: String,
}

impl fmt::Display for WriteError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "cannot write: {}", self.what)
    }
}

impl std::error::Error for WriteError {}

fn err<T>(what: impl Into<String>) -> Result<T, WriteError> {
    Err(WriteError { what: what.into() })
}

fn u16_of(n: usize, what: &str) -> Result<u16, WriteError> {
    u16::try_from(n).or_else(|_| err(format!("{what} ({n}) doesn't fit 16 bits")))
}

fn u32_of(n: usize, what: &str) -> Result<u32, WriteError> {
    u32::try_from(n).or_else(|_| err(format!("{what} ({n}) doesn't fit 32 bits")))
}

#[derive(Default)]
struct Out(Vec<u8>);

impl Out {
    fn u8(&mut self, v: u8) {
        self.0.push(v);
    }

    fn u16(&mut self, v: u16) {
        self.0.extend_from_slice(&v.to_le_bytes());
    }

    fn u32(&mut self, v: u32) {
        self.0.extend_from_slice(&v.to_le_bytes());
    }

    fn bytes(&mut self, v: &[u8]) {
        self.0.extend_from_slice(v);
    }

    fn pairs(&mut self, v: &[(u16, u16)]) {
        for &(a, b) in v {
            self.u16(a);
            self.u16(b);
        }
    }

    /// The string's UTF-16 units and a NUL.
    fn wz(&mut self, s: &str) {
        for u in s.encode_utf16().chain([0]) {
            self.u16(u);
        }
    }
}

/// Writes a whole file, sections laid out back to back after a canonical TOC.
pub fn write(pri: &Pri) -> Result<Vec<u8>, WriteError> {
    if pri.magic.len() != 8 {
        return err("a magic of 8 bytes");
    }
    let count = u16_of(pri.sections.len(), "the section count")?;
    let bodies = pri
        .sections
        .iter()
        .map(|s| Ok((s, section(s)?)))
        .collect::<Result<Vec<_>, WriteError>>()?;

    let first = HEADER_LEN + TOC_ENTRY_LEN * bodies.len();
    let mut toc = Out::default();
    let mut offset = 0;
    for (s, bytes) in &bodies {
        toc.bytes(&s.id);
        toc.u16(s.toc_flags);
        toc.u16(s.toc_section_flags);
        toc.u32(s.toc_qualifier);
        toc.u32(u32_of(offset, "a section's offset")?);
        toc.u32(u32_of(bytes.len(), "a section's length")?);
        offset += bytes.len();
    }
    let size = u32_of(first + offset + FILE_FOOTER_LEN, "the file size")?;

    let mut o = Out::default();
    o.bytes(pri.magic.as_bytes());
    o.u16(pri.header_u8);
    o.u16(pri.header_u10);
    o.u32(size);
    o.u32(HEADER_LEN as u32);
    o.u32(u32_of(first, "the first section's offset")?);
    o.u16(count);
    o.u16(pri.header_u26);
    o.u32(pri.header_u28);
    o.bytes(&toc.0);
    for (_, bytes) in &bodies {
        o.bytes(bytes);
    }
    o.u32(FILE_FOOTER);
    o.u32(size);
    o.bytes(pri.magic.as_bytes());
    Ok(o.0)
}

/// A section: header, body, footer.
fn section(s: &Section) -> Result<Vec<u8>, WriteError> {
    let body = match &s.body {
        Body::Descriptor(d) => descriptor(d)?,
        Body::Schema(None) => Vec::new(),
        Body::Schema(Some(h)) => schema(h)?,
        Body::Decisions(d) => decn_info(d)?,
        Body::Map(m) => res_map(m)?,
        Body::Data(d) => data_item(d)?,
        Body::Raw(b) => b.clone(),
    };
    let len = u32_of(SECTION_HEADER_LEN + body.len() + 8, "a section's length")?;
    let mut o = Out::default();
    o.bytes(&s.id);
    o.u32(s.header_qualifier);
    o.u16(s.header_flags);
    o.u16(s.header_section_flags);
    o.u32(len);
    o.u32(s.header_u28);
    o.bytes(&body);
    o.u32(SECTION_FOOTER);
    o.u32(len);
    Ok(o.0)
}

fn descriptor(d: &Descriptor) -> Result<Vec<u8>, WriteError> {
    let mut o = Out::default();
    o.u16(d.flags);
    o.u16(d.included_file_list);
    o.u16(d.u4);
    o.u16(u16_of(d.schemas.len(), "the schema count")?);
    o.u16(u16_of(d.decn_infos.len(), "the decision info count")?);
    o.u16(u16_of(d.res_maps.len(), "the resource map count")?);
    o.u16(d.primary_map);
    o.u16(u16_of(d.file_lists.len(), "the file list count")?);
    o.u16(u16_of(d.data_items.len(), "the data item count")?);
    o.u16(d.u18);
    for list in [
        &d.schemas,
        &d.decn_infos,
        &d.res_maps,
        &d.file_lists,
        &d.data_items,
    ] {
        for &i in list {
            o.u16(i);
        }
    }
    o.bytes(&d.tail);
    Ok(o.0)
}

/// The name blocks as makepri lays them out: scope names by index, then item names by index,
/// ASCII ones in the ASCII block (after its leading NUL) when the schema has one. Returns the
/// blocks and each node's (offset, is it ASCII).
type NameBlocks = (Vec<u8>, Vec<u8>, Vec<(u32, bool)>);

fn name_blocks(h: &Schema, has_ascii_block: bool) -> Result<NameBlocks, WriteError> {
    let mut uni = Out::default();
    let mut ascii = if has_ascii_block { vec![0] } else { Vec::new() };
    let mut placed = vec![(0, false); h.nodes.len()];
    let order = h
        .scopes
        .iter()
        .map(|s| s.node)
        .chain(h.items.iter().copied());
    for i in order {
        let Some(n) = h.nodes.get(i as usize) else {
            return err(format!("a scope or item node ({i}) inside the nodes"));
        };
        if n.name.is_empty() {
            continue;
        }
        if n.name.contains('\0') {
            return err("a name without a NUL");
        }
        placed[i as usize] = if has_ascii_block && n.name.is_ascii() {
            let at = u32_of(ascii.len(), "an ASCII name's offset")?;
            ascii.extend_from_slice(n.name.as_bytes());
            ascii.push(0);
            (at, true)
        } else {
            let at = u32_of(uni.0.len() / 2, "a name's offset")?;
            uni.wz(&n.name);
            (at, false)
        };
    }
    Ok((uni.0, ascii, placed))
}

fn schema(h: &Schema) -> Result<Vec<u8>, WriteError> {
    let has_ascii_block = h.hnames_id.as_ref() == Some(HNAMESX_ID);
    if h.extended != h.hnames_id.is_some() {
        return err("an extended schema, and only it, to have a names-block identifier");
    }
    let (uni, ascii, placed) = name_blocks(h, has_ascii_block)?;
    let full_names = h.full_names();
    let mut max_full_path = 0;
    let mut full_lens = Vec::with_capacity(full_names.len());
    for f in &full_names {
        let len = u16_of(f.encode_utf16().count(), "a full path's length")?;
        max_full_path = max_full_path.max(len);
        full_lens.push(len);
    }

    let mut o = Out::default();
    o.u16(1);
    o.u16(u16_of(
        h.unique_name.encode_utf16().count() + 1,
        "the unique name",
    )?);
    o.u16(u16_of(h.name.encode_utf16().count() + 1, "the name")?);
    o.u16(0);
    if let Some(id) = &h.hnames_id {
        o.bytes(id);
    }
    o.u16(h.major);
    o.u16(h.minor);
    o.u32(0);
    o.u32(h.checksum());
    o.u32(u32_of(h.scopes.len(), "the scope count")?);
    o.u32(u32_of(h.items.len(), "the item count")?);
    if h.unique_name.contains('\0') || h.name.contains('\0') {
        return err("names without a NUL");
    }
    o.wz(&h.unique_name);
    o.wz(&h.name);
    o.u16(0);

    // The names block, whose total is padded to 8 from its own start.
    let mut b = Out::default();
    b.u16(max_full_path);
    b.u16(0);
    b.u32(u32_of(h.nodes.len(), "the node count")?);
    b.u32(u32_of(h.scopes.len(), "the scope count")?);
    b.u32(u32_of(h.items.len(), "the item count")?);
    b.u32(u32_of(uni.len() / 2, "the Unicode block")?);
    b.u32(0); // the total, once the length is known
    if has_ascii_block {
        b.u32(u32_of(ascii.len(), "the ASCII block")?);
    }
    for (i, n) in h.nodes.iter().enumerate() {
        let (offset, is_ascii) = placed[i];
        if offset >> 16 > 0xF {
            return err("a name offset of at most 20 bits");
        }
        let name_len = n.name.chars().count();
        let flags = (if n.is_scope { 0x10 } else { 0 })
            | (if is_ascii { 0x20 } else { 0 })
            | (n.flags_hi & 0xC0)
            | (offset >> 16) as u8;
        b.u16(n.parent);
        b.u16(full_lens[i]);
        b.u16(first_letter(&n.name));
        b.u8(u8::try_from(name_len).unwrap_or(0));
        b.u8(flags);
        b.u16(offset as u16);
        b.u16(n.index);
    }
    for s in &h.scopes {
        b.u16(s.node);
        b.u16(s.children);
        b.u16(s.first_child);
        b.u16(0);
    }
    for &i in &h.items {
        b.u16(i);
    }
    b.bytes(&uni);
    b.bytes(&ascii);
    let used = b.0.len();
    let padded = used + (used.wrapping_neg() % 8).min(h.pad.len());
    b.0[20..24].copy_from_slice(&u32_of(padded, "the names block")?.to_le_bytes());
    o.bytes(&b.0);
    o.bytes(&h.pad);
    Ok(o.0)
}

/// The name's first character, uppercased, as a UTF-16 unit; 0 for none.
fn first_letter(name: &str) -> u16 {
    let Some(c) = name.chars().next() else {
        return 0;
    };
    let mut up = c.to_uppercase();
    let c = match (up.next(), up.next()) {
        (Some(u), None) => u,
        _ => c,
    };
    let mut unit = [0; 2];
    c.encode_utf16(&mut unit)[0]
}

fn decn_info(d: &DecnInfo) -> Result<Vec<u8>, WriteError> {
    if !d.values.len().is_multiple_of(2) {
        return err("a value block of whole UTF-16 units");
    }
    let mut o = Out::default();
    o.u16(u16_of(d.distinct.len(), "the distinct qualifier count")?);
    o.u16(u16_of(d.quals.len(), "the qualifier count")?);
    o.u16(u16_of(d.sets.len(), "the qualifier set count")?);
    o.u16(u16_of(d.decisions.len(), "the decision count")?);
    o.u16(u16_of(d.index_table.len(), "the index table")?);
    o.u16(u16_of(d.values.len() / 2, "the value block")?);
    o.pairs(&d.decisions);
    o.pairs(&d.sets);
    for q in &d.quals {
        o.u16(q.distinct);
        o.u16(q.priority);
        o.u16(q.fallback);
        o.u16(q.reserved);
    }
    for x in &d.distinct {
        o.u16(x.env);
        o.u16(x.qualifier_type);
        o.u16(x.op);
        o.u16(x.value_type);
        o.u32(x.value_offset);
    }
    for &i in &d.index_table {
        o.u16(i);
    }
    o.bytes(&d.values);
    o.bytes(&d.pad);
    Ok(o.0)
}

fn res_map(m: &ResMap) -> Result<Vec<u8>, WriteError> {
    let mut o = Out::default();
    o.u16(u16_of(m.env.len(), "the environment")?);
    o.u16(m.env_count);
    o.u16(m.schema_section);
    o.u16(u16_of(m.schema_ref.len(), "the schema reference")?);
    o.u16(m.decn_section);
    o.u16(u16_of(m.types.len(), "the type count")?);
    o.u16(u16_of(m.item_to_group.len(), "the item-to-group count")?);
    o.u16(u16_of(m.groups.len(), "the group count")?);
    o.u32(u32_of(m.infos.len(), "the info count")?);
    o.u32(u32_of(m.candidates.len(), "the candidate count")?);
    o.u32(u32_of(m.data.len(), "the data length")?);
    o.u32(u32_of(m.large.len(), "the large block")?);
    o.bytes(&m.env);
    o.bytes(&m.schema_ref);
    for &(a, b) in &m.types {
        o.u32(a);
        o.u32(b);
    }
    o.pairs(&m.item_to_group);
    o.pairs(&m.groups);
    o.pairs(&m.infos);
    o.bytes(&m.large);
    for c in &m.candidates {
        match *c {
            Candidate::Ref {
                value_type,
                reserved,
                item,
                section,
            } => {
                o.u8(1);
                o.u8(value_type);
                o.u16(reserved);
                o.u16(item);
                o.u16(section);
            }
            Candidate::Embedded {
                value_type,
                reserved,
                value,
            } => {
                o.u8(0);
                o.u8(value_type);
                o.u16(reserved);
                o.u32(value);
            }
        }
    }
    o.bytes(&m.data);
    o.bytes(&m.pad);
    Ok(o.0)
}

/// Strings then blobs, each at a 4-byte boundary, the body ending on 8 bytes.
fn data_item(d: &DataItem) -> Result<Vec<u8>, WriteError> {
    let n_strs = d.strs.len();
    let mut data = Vec::new();
    let mut strs = Vec::new();
    let mut blobs = Vec::new();
    for (i, v) in d.values().into_iter().enumerate() {
        let at = data.len();
        if i < n_strs {
            let at = u16_of(at, "a string's offset")?;
            strs.push((at, u16_of(v.len(), "a string's length")?));
        } else {
            let at = u32_of(at, "a blob's offset")?;
            blobs.push((at, u32_of(v.len(), "a blob's length")?));
        }
        data.extend_from_slice(v);
        data.resize(data.len().next_multiple_of(4), 0);
    }
    let fixed = 12 + 4 * strs.len() + 8 * blobs.len();
    data.resize(data.len() + (fixed + data.len()).wrapping_neg() % 8, 0);

    let mut o = Out::default();
    o.u32(d.u0);
    o.u16(u16_of(strs.len(), "the string count")?);
    o.u16(u16_of(blobs.len(), "the blob count")?);
    o.u32(u32_of(data.len(), "the data length")?);
    o.pairs(&strs);
    for &(a, b) in &blobs {
        o.u32(a);
        o.u32(b);
    }
    o.bytes(&data);
    o.bytes(&d.pad);
    Ok(o.0)
}
