//! `pri dump <file.pri>`: every section, the names, and each item's candidates.

use std::fmt::Write as _;
use std::io::{ErrorKind, Write as _};
use std::process::ExitCode;

use pri::{Body, Candidate, Pri, Section};

const USAGE: &str = "usage: pri dump <file.pri>";
/// Byte runs longer than this are printed abbreviated.
const HEX_LIMIT: usize = 48;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.as_slice() {
        [cmd, path] if cmd == "dump" => dump_file(path),
        _ => {
            eprintln!("{USAGE}");
            ExitCode::from(2)
        }
    }
}

fn dump_file(path: &str) -> ExitCode {
    let data = match std::fs::read(path) {
        Ok(d) => d,
        Err(e) => {
            eprintln!("pri: {path}: {e}");
            return ExitCode::FAILURE;
        }
    };
    let file = match pri::read(&data) {
        Ok(f) => f,
        Err(e) => {
            eprintln!("pri: {path}: {e}");
            return ExitCode::FAILURE;
        }
    };
    let (mut text, all_ok) = dump(&file);
    let listing = file.listing();
    if let Ok(listing) = &listing {
        if !listing.is_empty() {
            text.push_str("\ncandidates\n");
            text.push_str(listing);
        }
    }
    // A closed pipe (`pri dump x | head`) is the reader's choice, not a failure.
    let mut out = std::io::stdout().lock();
    if let Err(e) = out.write_all(text.as_bytes()).and_then(|()| out.flush()) {
        return if e.kind() == ErrorKind::BrokenPipe {
            ExitCode::SUCCESS
        } else {
            eprintln!("pri: {path}: {e}");
            ExitCode::FAILURE
        };
    }
    if let Err(e) = listing {
        eprintln!("pri: {path}: candidates: {e}");
        return ExitCode::FAILURE;
    }
    if all_ok {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}

/// Long runs as the first bytes in hex and the length.
fn hex(b: &[u8]) -> String {
    let h = |b: &[u8]| b.iter().fold(String::new(), |s, x| s + &format!("{x:02x}"));
    if b.len() > HEX_LIMIT {
        format!("{}... ({} bytes)", h(&b[..HEX_LIMIT]), b.len())
    } else {
        h(b)
    }
}

/// Everything but the candidates; the flag says whether every checksum matched.
fn dump(file: &Pri) -> (String, bool) {
    let mut o = String::new();
    let mut all_ok = true;
    let _ = writeln!(
        o,
        "header magic={:?} u8={} u10={} u26={:#x} u28={} sections={}",
        file.magic,
        file.header_u8,
        file.header_u10,
        file.header_u26,
        file.header_u28,
        file.sections.len()
    );
    for (i, s) in file.sections.iter().enumerate() {
        let _ = writeln!(
            o,
            "\n#{i} {:?} toc(flags={:#x} sflags={:#x} qual={:#x}) hdr(qual={:#x} flags={:#x} sflags={:#x} u28={})",
            s.id_text(),
            s.toc_flags,
            s.toc_section_flags,
            s.toc_qualifier,
            s.header_qualifier,
            s.header_flags,
            s.header_section_flags,
            s.header_u28
        );
        all_ok &= dump_body(&mut o, s);
    }
    (o, all_ok)
}

fn dump_body(o: &mut String, s: &Section) -> bool {
    let mut ok = true;
    match &s.body {
        Body::Descriptor(d) => {
            let _ = writeln!(
                o,
                "  flags={:#x} included_file_list={:#x} u4={} primary_map={} u18={}",
                d.flags, d.included_file_list, d.u4, d.primary_map, d.u18
            );
            let _ = writeln!(
                o,
                "  schemas={:?} decn_infos={:?} res_maps={:?} file_lists={:?} data_items={:?}",
                d.schemas, d.decn_infos, d.res_maps, d.file_lists, d.data_items
            );
            let _ = writeln!(o, "  tail: {}", hex(&d.tail));
        }
        Body::Schema(None) => {
            let _ = writeln!(o, "  empty schema");
        }
        Body::Schema(Some(h)) => {
            let _ = writeln!(
                o,
                "  extended={} hnames_id={:?} version={}.{} unique_name={:?} name={:?}",
                h.extended,
                h.hnames_id.as_ref().map(pri::id_text),
                h.major,
                h.minor,
                h.unique_name,
                h.name
            );
            let _ = writeln!(
                o,
                "  max_full_path={} total={} scopes={} items={} names_uni={} bytes names_ascii={} bytes pad: {}",
                h.max_full_path,
                h.total,
                h.scopes.len(),
                h.items.len(),
                h.names_uni.len(),
                h.names_ascii.len(),
                hex(&h.pad)
            );
            for (i, n) in h.nodes.iter().enumerate() {
                let first = char::from_u32(n.first as u32).filter(|&c| c != '\0');
                let _ = writeln!(
                    o,
                    "  node {i}: {} idx={} parent={} full_len={} first={:?} name_len={} {} off={} hi={:#x} name={:?} full={:?}",
                    if n.is_scope { "scope" } else { "item " },
                    n.index,
                    n.parent,
                    n.full_len,
                    first.map(String::from).unwrap_or_default(),
                    n.name_len,
                    if n.is_ascii { "ascii" } else { "uni" },
                    n.offset,
                    n.flags_hi,
                    n.name,
                    h.full_name(i)
                );
            }
            let computed = h.checksum();
            let matches = computed == h.checksum;
            ok &= matches;
            let _ = writeln!(
                o,
                "  checksum computed {computed:#010x} stored {:#010x} {}",
                h.checksum,
                if matches { "ok" } else { "MISMATCH" }
            );
        }
        Body::Decisions(d) => {
            let _ = writeln!(o, "  decisions: {:?}", d.decisions);
            let _ = writeln!(o, "  sets: {:?}", d.sets);
            let quals: Vec<_> = d
                .quals
                .iter()
                .map(|q| (q.distinct, q.priority, q.fallback, q.reserved))
                .collect();
            let _ = writeln!(o, "  quals: {quals:?}");
            for (i, x) in d.distinct.iter().enumerate() {
                let _ = writeln!(
                    o,
                    "  distinct {i}: env={} {}={:?} op={} value_type={} offset={}",
                    x.env,
                    x.type_name(),
                    x.value,
                    x.op,
                    x.value_type,
                    x.value_offset
                );
            }
            let _ = writeln!(o, "  index_table: {:?}", d.index_table);
            let _ = writeln!(o, "  values: {}", hex(&d.values));
            let _ = writeln!(o, "  pad: {}", hex(&d.pad));
        }
        Body::Map(m) => {
            let _ = writeln!(
                o,
                "  v2={} env={} env_count={} schema_section={} schema_ref={} decn_section={}",
                m.v2,
                hex(&m.env),
                m.env_count,
                m.schema_section,
                hex(&m.schema_ref),
                m.decn_section
            );
            let _ = writeln!(o, "  types: {:?}", m.types);
            let _ = writeln!(o, "  item_to_group: {:?}", m.item_to_group);
            let _ = writeln!(o, "  groups: {:?}", m.groups);
            let _ = writeln!(o, "  infos: {:?}", m.infos);
            let _ = writeln!(o, "  large: {}", hex(&m.large));
            let _ = writeln!(o, "  candidates: {}", m.candidates.len());
            for (i, c) in m.candidates.iter().enumerate() {
                let _ = match c {
                    Candidate::Ref {
                        value_type,
                        reserved,
                        item,
                        section,
                    } => writeln!(
                        o,
                        "    {i}: ref type={value_type} reserved={reserved} item={item} section={section}"
                    ),
                    Candidate::Embedded {
                        value_type,
                        reserved,
                        value,
                    } => writeln!(
                        o,
                        "    {i}: embedded type={value_type} reserved={reserved} value={value}"
                    ),
                };
            }
            let _ = writeln!(o, "  data: {}", hex(&m.data));
            let _ = writeln!(o, "  pad: {}", hex(&m.pad));
        }
        Body::Data(d) => {
            let _ = writeln!(o, "  u0={} strs={:?} blobs={:?}", d.u0, d.strs, d.blobs);
            let _ = writeln!(o, "  pad: {}", hex(&d.pad));
            for (i, v) in d.values().iter().enumerate() {
                let _ = writeln!(o, "  value {i}: {}", value_text(v));
            }
        }
        Body::Raw(b) => {
            let _ = writeln!(o, "  raw body: {}", hex(b));
        }
    }
    ok
}

/// A value as text when it is ASCII or UTF-16, hex when it is neither.
fn value_text(v: &[u8]) -> String {
    let trimmed = v.strip_suffix(&[0]).unwrap_or(v);
    if !trimmed.is_empty() && trimmed.iter().all(|b| (0x20..0x7F).contains(b)) {
        return format!("{:?}", String::from_utf8_lossy(trimmed));
    }
    let units: Vec<u16> = v
        .as_chunks::<2>()
        .0
        .iter()
        .map(|c| u16::from_le_bytes([c[0], c[1]]))
        .collect();
    match String::from_utf16(&units) {
        Ok(s) if v.len().is_multiple_of(2) => format!("{:?}", s.trim_end_matches('\0')),
        _ => hex(v),
    }
}
