//! Bytes to model. Every read is bounds-checked; malformed input is an `Error`, never a panic.

use crate::{
    id_text, Body, Candidate, DataItem, DecnInfo, Descriptor, Distinct, Error, Node, Pri,
    Qualifier, ResMap, Schema, Scope, Section,
};

const SECTION_FOOTER: u32 = 0xDEF5_FADE;
const FILE_FOOTER: u32 = 0xDEFF_FADE;
const HEADER_LEN: usize = 32;
const SECTION_HEADER_LEN: usize = 32;
const SECTION_FOOTER_LEN: usize = 8;
const MAGIC: &[u8; 8] = b"mrm_pri2";
/// Longest qualifier value we decode (UTF-16 units); real ones are a few characters.
const MAX_QUALIFIER_VALUE_UNITS: usize = 1024;
/// Most units all the long-named nodes together may make us scan.
const NAME_SCAN_BUDGET: usize = 16 << 20;
const HNAMESX_ID: &[u8; 16] = b"[def_hnamesx]  \0";

/// A cursor over `data`, whose first byte sits at `base` in the file (for error offsets).
struct Reader<'a> {
    data: &'a [u8],
    pos: usize,
    base: usize,
}

impl<'a> Reader<'a> {
    fn new(data: &'a [u8], base: usize) -> Self {
        Reader { data, pos: 0, base }
    }

    fn fail<T>(&self, expected: &str) -> Result<T, Error> {
        Err(Error {
            offset: self.base + self.pos,
            expected: expected.to_string(),
        })
    }

    fn bytes(&mut self, n: usize) -> Result<&'a [u8], Error> {
        let end = self.pos.checked_add(n).filter(|&e| e <= self.data.len());
        match end {
            Some(end) => {
                let out = &self.data[self.pos..end];
                self.pos = end;
                Ok(out)
            }
            None => self.fail(&format!("{n} more bytes")),
        }
    }

    fn array<const N: usize>(&mut self) -> Result<[u8; N], Error> {
        let mut out = [0; N];
        out.copy_from_slice(self.bytes(N)?);
        Ok(out)
    }

    fn u8(&mut self) -> Result<u8, Error> {
        Ok(self.bytes(1)?[0])
    }

    fn u16(&mut self) -> Result<u16, Error> {
        Ok(u16::from_le_bytes(self.array()?))
    }

    fn u32(&mut self) -> Result<u32, Error> {
        Ok(u32::from_le_bytes(self.array()?))
    }

    fn rest(&mut self) -> Vec<u8> {
        let out = self.data[self.pos..].to_vec();
        self.pos = self.data.len();
        out
    }

    fn zero16(&mut self) -> Result<(), Error> {
        let at = self.pos;
        if self.u16()? != 0 {
            self.pos = at;
            return self.fail("0");
        }
        Ok(())
    }

    fn zero32(&mut self) -> Result<(), Error> {
        let at = self.pos;
        if self.u32()? != 0 {
            self.pos = at;
            return self.fail("0");
        }
        Ok(())
    }

    fn u16s(&mut self, n: usize) -> Result<Vec<u16>, Error> {
        (0..n).map(|_| self.u16()).collect()
    }

    fn pairs(&mut self, n: usize) -> Result<Vec<(u16, u16)>, Error> {
        (0..n).map(|_| Ok((self.u16()?, self.u16()?))).collect()
    }

    /// `cch` UTF-16 units, terminator included; the string must not hold a NUL of its own.
    fn wstr(&mut self, cch: usize) -> Result<String, Error> {
        let at = self.pos;
        let raw = self.bytes(cch.saturating_mul(2))?;
        let units = units(raw);
        let decoded = match units.split_last() {
            Some((0, body)) => String::from_utf16(body).ok().filter(|s| !s.contains('\0')),
            _ => None,
        };
        match decoded {
            Some(s) => Ok(s),
            None => {
                self.pos = at;
                self.fail("a valid UTF-16 string of the stated length, ending in one NUL")
            }
        }
    }

    /// The UTF-16 units of the string at `rel` units into the block of `len` bytes starting at `start`,
    /// ending in a NUL inside the block and at most `max_units` long.
    fn wcharz_in(
        &self,
        start: usize,
        len: usize,
        rel: usize,
        max_units: usize,
    ) -> Result<Vec<u16>, Error> {
        let err = || Error {
            offset: self.base + start + rel.saturating_mul(2),
            expected: "a UTF-16 string ending in NUL inside its block".into(),
        };
        let block = self.data.get(start..start + len).ok_or_else(err)?;
        let tail = block.get(rel.saturating_mul(2)..).ok_or_else(err)?;
        let units: Vec<u16> = units(tail)
            .into_iter()
            .take(max_units.saturating_add(1))
            .take_while(|&u| u != 0)
            .collect();
        let terminated = tail.len() / 2 > units.len() && units.len() <= max_units;
        if !terminated {
            return Err(err());
        }
        Ok(units)
    }

    /// The same for a NUL-terminated ASCII string.
    fn charz_in(
        &self,
        start: usize,
        len: usize,
        rel: usize,
        max_len: usize,
    ) -> Result<String, Error> {
        let err = || Error {
            offset: self.base + start + rel,
            expected: "an ASCII string ending in NUL inside its block".into(),
        };
        let block = self.data.get(start..start + len).ok_or_else(err)?;
        let tail = block.get(rel..).ok_or_else(err)?;
        let window = &tail[..tail.len().min(max_len.saturating_add(1))];
        let n = window.iter().position(|&b| b == 0).ok_or_else(err)?;
        if !window[..n].is_ascii() {
            return Err(err());
        }
        Ok(String::from_utf8_lossy(&window[..n]).into_owned())
    }
}

fn units(raw: &[u8]) -> Vec<u16> {
    raw.as_chunks::<2>()
        .0
        .iter()
        .map(|c| u16::from_le_bytes([c[0], c[1]]))
        .collect()
}

/// Reads a whole file.
pub fn read(data: &[u8]) -> Result<Pri, Error> {
    let mut r = Reader::new(data, 0);
    let magic_raw = r.bytes(8)?;
    if magic_raw != MAGIC {
        r.pos = 0;
        return r.fail("the magic mrm_pri2");
    }
    let magic = String::from_utf8_lossy(magic_raw).into_owned();
    let header_u8 = r.u16()?;
    let header_u10 = r.u16()?;
    let size_at = r.pos;
    let size = r.u32()? as usize;
    if size != data.len() {
        r.pos = size_at;
        return r.fail(&format!("the file size {} here", data.len()));
    }
    let toc_offset = r.u32()? as usize;
    let first = r.u32()? as usize;
    let count = r.u16()? as usize;
    let header_u26 = r.u16()?;
    let header_u28 = r.u32()?;

    // The footer: file footer magic, the size again, and the magic.
    let mut f = Reader::new(data, 0);
    f.pos = match data.len().checked_sub(16) {
        Some(p) if p >= HEADER_LEN => p,
        _ => return r.fail("a file footer"),
    };
    let footer_at = f.pos;
    let ok = f.u32()? == FILE_FOOTER && f.u32()? as usize == data.len() && f.bytes(8)? == magic_raw;
    if !ok {
        f.pos = footer_at;
        return f.fail("the file footer (0xDEFFFADE, size, magic)");
    }

    // The TOC and sections are read wherever the header says; the writer lays them out
    // canonically, so a round trip is what flags a file that isn't.
    r.pos = toc_offset;
    let mut tocs = Vec::new();
    for _ in 0..count {
        let id = r.array::<16>()?;
        let flags = r.u16()?;
        let section_flags = r.u16()?;
        let qualifier = r.u32()?;
        let off = r.u32()? as usize;
        let len = r.u32()? as usize;
        tocs.push((id, flags, section_flags, qualifier, off, len));
    }

    let mut sections = Vec::new();
    for (id, toc_flags, toc_section_flags, toc_qualifier, off, len) in tocs {
        let mut s = Reader::new(data, 0);
        s.pos = first.saturating_add(off);
        let start = s.pos;
        if len < SECTION_HEADER_LEN + SECTION_FOOTER_LEN {
            return s.fail(&format!(
                "a section of at least {} bytes",
                SECTION_HEADER_LEN + SECTION_FOOTER_LEN
            ));
        }
        let raw = s.bytes(len)?;
        let mut h = Reader::new(raw, start);
        if h.array::<16>()? != id {
            h.pos = 0;
            return h.fail("the section header to repeat the TOC's identifier");
        }
        let header_qualifier = h.u32()?;
        let header_flags = h.u16()?;
        let header_section_flags = h.u16()?;
        let len_at = h.pos;
        if h.u32()? as usize != len {
            h.pos = len_at;
            return h.fail(&format!("the section's length {len}"));
        }
        let header_u28 = h.u32()?;
        let mut foot = Reader::new(raw, start);
        foot.pos = len - SECTION_FOOTER_LEN;
        if foot.u32()? != SECTION_FOOTER || foot.u32()? as usize != len {
            foot.pos = len - SECTION_FOOTER_LEN;
            return foot.fail("the section footer (0xDEF5FADE, length)");
        }
        let body_raw = &raw[SECTION_HEADER_LEN..len - SECTION_FOOTER_LEN];
        let body = parse_body(&id, body_raw, start + SECTION_HEADER_LEN)?;
        sections.push(Section {
            id,
            toc_flags,
            toc_section_flags,
            toc_qualifier,
            header_qualifier,
            header_flags,
            header_section_flags,
            header_u28,
            body,
        });
    }
    Ok(Pri {
        magic,
        header_u8,
        header_u10,
        header_u26,
        header_u28,
        sections,
    })
}

fn parse_body(id: &[u8; 16], b: &[u8], base: usize) -> Result<Body, Error> {
    let kind = id_text(id);
    let mut r = Reader::new(b, base);
    Ok(match kind.as_str() {
        "[mrm_pridescex]" => Body::Descriptor(descriptor(&mut r)?),
        "[mrm_hschema]" => Body::Schema(schema(&mut r, false)?),
        "[mrm_hschemaex]" => Body::Schema(schema(&mut r, true)?),
        "[mrm_decn_info]" => Body::Decisions(decn_info(&mut r)?),
        "[mrm_res_map__]" => Body::Map(res_map(&mut r, false)?),
        "[mrm_res_map2_]" => Body::Map(res_map(&mut r, true)?),
        "[mrm_dataitem]" => Body::Data(data_item(&mut r)?),
        _ => Body::Raw(b.to_vec()),
    })
}

fn descriptor(r: &mut Reader) -> Result<Descriptor, Error> {
    let flags = r.u16()?;
    let included_file_list = r.u16()?;
    let u4 = r.u16()?;
    let n_hs = r.u16()?;
    let n_di = r.u16()?;
    let n_rm = r.u16()?;
    let primary_map = r.u16()?;
    let n_rf = r.u16()?;
    let n_dt = r.u16()?;
    let u18 = r.u16()?;
    Ok(Descriptor {
        flags,
        included_file_list,
        u4,
        primary_map,
        u18,
        schemas: r.u16s(n_hs as usize)?,
        decn_infos: r.u16s(n_di as usize)?,
        res_maps: r.u16s(n_rm as usize)?,
        file_lists: r.u16s(n_rf as usize)?,
        data_items: r.u16s(n_dt as usize)?,
        tail: r.rest(),
    })
}

fn schema(r: &mut Reader, extended: bool) -> Result<Option<Schema>, Error> {
    if r.data.is_empty() {
        return Ok(None);
    }
    let at = r.pos;
    if r.u16()? != 1 {
        r.pos = at;
        return r.fail("1");
    }
    let cch_unique = r.u16()? as usize;
    let cch_name = r.u16()? as usize;
    r.zero16()?;
    let hnames_id = if extended {
        Some(r.array::<16>()?)
    } else {
        None
    };
    let has_ascii_block = hnames_id.as_ref() == Some(HNAMESX_ID);
    let major = r.u16()?;
    let minor = r.u16()?;
    r.zero32()?;
    let checksum = r.u32()?;
    let n_scopes = r.u32()? as usize;
    let n_items = r.u32()? as usize;
    let unique_name = r.wstr(cch_unique)?;
    let name = r.wstr(cch_name)?;
    r.zero16()?;

    let max_full_path = r.u16()?;
    r.zero16()?;
    let counts_at = r.pos;
    let n_names = r.u32()? as usize;
    let n_scopes2 = r.u32()? as usize;
    let n_items2 = r.u32()? as usize;
    if n_names != n_scopes.saturating_add(n_items) || n_scopes2 != n_scopes || n_items2 != n_items {
        r.pos = counts_at;
        return r.fail("the name counts to equal the scope and item counts");
    }
    let cch_uni = r.u32()? as usize;
    let total = r.u32()?;
    let cb_ascii = if has_ascii_block {
        r.u32()? as usize
    } else {
        0
    };

    let mut nodes = Vec::new();
    for _ in 0..n_names {
        let parent = r.u16()?;
        let full_len = r.u16()?;
        let first = r.u16()?;
        let name_len = r.u8()?;
        let flags = r.u8()?;
        let off_lo = r.u16()?;
        let index = r.u16()?;
        nodes.push(Node {
            parent,
            full_len,
            first,
            name_len,
            is_scope: flags & 0x10 != 0,
            is_ascii: flags & 0x20 != 0,
            flags_hi: flags & 0xC0,
            offset: off_lo as u32 | ((flags as u32 & 0xF) << 16),
            index,
            name: String::new(),
        });
    }
    let mut scopes = Vec::new();
    for _ in 0..n_scopes {
        let node = r.u16()?;
        let children = r.u16()?;
        let first_child = r.u16()?;
        r.zero16()?;
        scopes.push(Scope {
            node,
            children,
            first_child,
        });
    }
    let items = r.u16s(n_items)?;
    let uni_start = r.pos;
    let names_uni = r.bytes(cch_uni.saturating_mul(2))?.to_vec();
    let ascii_start = r.pos;
    let names_ascii = r.bytes(cb_ascii)?.to_vec();
    let pad = r.rest();

    // Each name is looked up inside its own block. A name is no longer than its node says
    // (`name_len`, or the path's length when that is 0), which also bounds the scan.
    let mut budget = NAME_SCAN_BUDGET;
    for n in &mut nodes {
        if n.full_len == 0 {
            continue;
        }
        let max = match n.name_len {
            0 => {
                budget = match budget.checked_sub(n.full_len as usize) {
                    Some(b) => b,
                    None => return r.fail("fewer long names (a name scan over budget)"),
                };
                n.full_len as usize
            }
            len => len as usize * 2,
        };
        let offset = n.offset as usize;
        n.name = if n.is_ascii {
            r.charz_in(ascii_start, cb_ascii, offset, max)?
        } else {
            let units = r.wcharz_in(uni_start, names_uni.len(), offset, max)?;
            match String::from_utf16(&units) {
                Ok(s) => s,
                Err(_) => {
                    r.pos = uni_start + offset * 2;
                    return r.fail("a valid UTF-16 name");
                }
            }
        };
        if n.name_len != 0 && n.name.chars().count() != n.name_len as usize {
            r.pos = if n.is_ascii {
                ascii_start + offset
            } else {
                uni_start + offset * 2
            };
            return r.fail(&format!("a name of {} characters", n.name_len));
        }
    }
    check_parents(&nodes).or_else(|i| {
        r.pos = counts_at;
        r.fail(&format!("node {i} to reach the root without a cycle"))
    })?;
    Ok(Some(Schema {
        extended,
        hnames_id,
        major,
        minor,
        checksum,
        unique_name,
        name,
        max_full_path,
        total,
        nodes,
        scopes,
        items,
        names_uni,
        names_ascii,
        pad,
    }))
}

/// Every node's parent chain must end at the root (a self-parent or 0xFFFF) with no cycle.
/// Returns the first node that doesn't.
fn check_parents(nodes: &[Node]) -> Result<(), usize> {
    #[derive(Clone, Copy, PartialEq)]
    enum Mark {
        Unseen,
        OnPath,
        Reaches,
    }
    let mut marks = vec![Mark::Unseen; nodes.len()];
    for start in 0..nodes.len() {
        let mut path = Vec::new();
        let mut i = start;
        loop {
            match marks[i] {
                Mark::Reaches => break,
                Mark::OnPath => return Err(start),
                Mark::Unseen => {}
            }
            marks[i] = Mark::OnPath;
            path.push(i);
            let parent = nodes[i].parent;
            if parent as usize == i || parent == 0xFFFF {
                break;
            }
            if parent as usize >= nodes.len() {
                return Err(i);
            }
            i = parent as usize;
        }
        for i in path {
            marks[i] = Mark::Reaches;
        }
    }
    Ok(())
}

fn decn_info(r: &mut Reader) -> Result<DecnInfo, Error> {
    let n_distinct = r.u16()? as usize;
    let n_qual = r.u16()? as usize;
    let n_sets = r.u16()? as usize;
    let n_dec = r.u16()? as usize;
    let n_index = r.u16()? as usize;
    let cch_values = r.u16()? as usize;
    let decisions = r.pairs(n_dec)?;
    let sets = r.pairs(n_sets)?;
    let mut quals = Vec::new();
    for _ in 0..n_qual {
        quals.push(Qualifier {
            distinct: r.u16()?,
            priority: r.u16()?,
            fallback: r.u16()?,
            reserved: r.u16()?,
        });
    }
    let mut distinct = Vec::new();
    for _ in 0..n_distinct {
        distinct.push(Distinct {
            env: r.u16()?,
            qualifier_type: r.u16()?,
            op: r.u16()?,
            value_type: r.u16()?,
            value_offset: r.u32()?,
            value: String::new(),
        });
    }
    let index_table = r.u16s(n_index)?;
    let values_start = r.pos;
    let values = r.bytes(cch_values.saturating_mul(2))?.to_vec();
    let pad = r.rest();
    for d in &mut distinct {
        let units = r.wcharz_in(
            values_start,
            values.len(),
            d.value_offset as usize,
            MAX_QUALIFIER_VALUE_UNITS,
        )?;
        d.value = String::from_utf16_lossy(&units);
    }
    Ok(DecnInfo {
        decisions,
        sets,
        quals,
        distinct,
        index_table,
        values,
        pad,
    })
}

fn res_map(r: &mut Reader, v2: bool) -> Result<ResMap, Error> {
    let env_len = r.u16()? as usize;
    let env_count = r.u16()?;
    let schema_section = r.u16()?;
    let schema_ref_len = r.u16()? as usize;
    let decn_section = r.u16()?;
    let n_types = r.u16()? as usize;
    let n_i2g = r.u16()? as usize;
    let n_groups = r.u16()? as usize;
    let n_infos = r.u32()? as usize;
    let n_cands = r.u32()? as usize;
    let data_len = r.u32()? as usize;
    let large_len = r.u32()? as usize;
    let env = r.bytes(env_len)?.to_vec();
    let schema_ref = r.bytes(schema_ref_len)?.to_vec();
    let types = (0..n_types)
        .map(|_| Ok((r.u32()?, r.u32()?)))
        .collect::<Result<_, Error>>()?;
    let item_to_group = r.pairs(n_i2g)?;
    let groups = r.pairs(n_groups)?;
    let infos = r.pairs(n_infos)?;
    let large = r.bytes(large_len)?.to_vec();
    let mut candidates = Vec::new();
    for _ in 0..n_cands {
        let at = r.pos;
        candidates.push(match r.u8()? {
            1 => Candidate::Ref {
                value_type: r.u8()?,
                reserved: r.u16()?,
                item: r.u16()?,
                section: r.u16()?,
            },
            0 => Candidate::Embedded {
                value_type: r.u8()?,
                reserved: r.u16()?,
                value: r.u32()?,
            },
            _ => {
                r.pos = at;
                return r.fail("a candidate of type 0 or 1");
            }
        });
    }
    let data = r.bytes(data_len)?.to_vec();
    let pad = r.rest();
    Ok(ResMap {
        v2,
        env,
        env_count,
        schema_section,
        schema_ref,
        decn_section,
        types,
        item_to_group,
        groups,
        infos,
        large,
        candidates,
        data,
        pad,
    })
}

fn data_item(r: &mut Reader) -> Result<DataItem, Error> {
    let u0 = r.u32()?;
    let n_str = r.u16()? as usize;
    let n_blob = r.u16()? as usize;
    let total = r.u32()? as usize;
    let strs = r.pairs(n_str)?;
    let blobs: Vec<(u32, u32)> = (0..n_blob)
        .map(|_| Ok((r.u32()?, r.u32()?)))
        .collect::<Result<_, Error>>()?;
    let data_at = r.pos;
    let data = r.bytes(total)?.to_vec();
    let pad = r.rest();
    let in_range = |o: usize, n: usize| o.checked_add(n).is_some_and(|e| e <= data.len());
    let ranges = strs
        .iter()
        .map(|&(o, n)| (o as usize, n as usize))
        .chain(blobs.iter().map(|&(o, n)| (o as usize, n as usize)));
    for (o, n) in ranges {
        if !in_range(o, n) {
            r.pos = data_at;
            return r.fail(&format!("a value at {o}, {n} long, inside the data"));
        }
    }
    Ok(DataItem {
        u0,
        strs,
        blobs,
        data,
        pad,
    })
}
