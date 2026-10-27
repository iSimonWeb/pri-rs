//! Our own builds (committed) and, where the machine has them, Windows' PRIs (never committed).

use std::path::Path;

use pri::{Body, Pri};

/// Windows' PRIs above this size are skipped, to keep the test quick.
const SYSTEM_PRI_LIMIT: u64 = 3 * 1024 * 1024;

fn check(path: &Path) -> Pri {
    let data = std::fs::read(path).unwrap();
    let file = pri::read(&data).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    for s in &file.sections {
        if let Body::Schema(Some(h)) = &s.body {
            assert_eq!(h.checksum(), h.checksum, "{}", path.display());
        }
    }
    file.listing()
        .unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    file
}

fn golden(name: &str) -> Pri {
    check(
        &Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/golden")
            .join(name),
    )
}

#[test]
fn golden_udc() {
    let file = golden("udc.pri");
    assert!(file.sections.iter().any(|s| matches!(s.body, Body::Map(_))));
}

#[test]
fn golden_parental_controls() {
    golden("ParentalControls_cw5n1h2txyewy.pri");
}

#[test]
fn golden_add_suggested_folders() {
    golden("Microsoft.Windows.AddSuggestedFoldersToLibraryDialog_cw5n1h2txyewy.pri");
}

#[test]
fn system_apps_parse_and_checksum() {
    let Ok(dir) = std::fs::read_dir("/mnt/c/Windows/SystemApps") else {
        return;
    };
    for entry in dir.flatten() {
        let path = entry.path().join("resources.pri");
        if std::fs::metadata(&path).is_ok_and(|m| m.len() <= SYSTEM_PRI_LIMIT) {
            check(&path);
        }
    }
}

#[test]
fn malformed_input_is_an_error_not_a_panic() {
    let data =
        std::fs::read(Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/golden/udc.pri")).unwrap();
    for len in 0..data.len() {
        assert!(pri::read(&data[..len]).is_err());
    }
    // Flip each byte: any result is fine, a panic is not.
    for i in 0..data.len() {
        let mut broken = data.clone();
        broken[i] ^= 0xFF;
        let _ = pri::read(&broken).map(|f| f.listing());
    }
}

fn put16(b: &mut Vec<u8>, v: u16) {
    b.extend_from_slice(&v.to_le_bytes());
}

fn put32(b: &mut Vec<u8>, v: u32) {
    b.extend_from_slice(&v.to_le_bytes());
}

/// A node record: (parent, full_len, name_len, flags, offset, index).
type RawNode = (u16, u16, u8, u8, u16, u16);

/// A file of one compact `[mrm_hschema]` section: one scope (node 0, the root), the other
/// nodes items, and `uni` as the UTF-16 name block.
fn schema_file(nodes: &[RawNode], uni: &[u8]) -> Vec<u8> {
    let n_items = nodes.len() as u32 - 1;
    let mut b = Vec::new();
    for v in [1, 2, 2, 0] {
        put16(&mut b, v); // 1, the two name lengths in units, 0
    }
    put16(&mut b, 1); // major
    put16(&mut b, 0); // minor
    put32(&mut b, 0);
    put32(&mut b, 0); // checksum
    put32(&mut b, 1); // scopes
    put32(&mut b, n_items);
    b.extend_from_slice(&[b'a', 0, 0, 0, b'b', 0, 0, 0, 0, 0]); // unique name, name, NUL
    put16(&mut b, 1); // max full path
    put16(&mut b, 0);
    put32(&mut b, nodes.len() as u32);
    put32(&mut b, 1);
    put32(&mut b, n_items);
    put32(&mut b, uni.len() as u32 / 2);
    put32(&mut b, 0); // total
    for &(parent, full_len, name_len, flags, off, index) in nodes {
        put16(&mut b, parent);
        put16(&mut b, full_len);
        put16(&mut b, 0);
        b.extend_from_slice(&[name_len, flags]);
        put16(&mut b, off);
        put16(&mut b, index);
    }
    for v in [0, nodes.len() as u16 - 1, 1, 0] {
        put16(&mut b, v); // the root scope: node, children, first child, 0
    }
    for i in 1..nodes.len() as u16 {
        put16(&mut b, i);
    }
    b.extend_from_slice(uni);
    b.resize(b.len().div_ceil(8) * 8, 0);

    let id = *b"[mrm_hschema]\0\0\0";
    let section_len = 32 + b.len() as u32 + 8;
    let size = 64 + section_len + 16;
    let mut f = Vec::new();
    f.extend_from_slice(b"mrm_pri2");
    for v in [0, 1] {
        put16(&mut f, v);
    }
    for v in [size, 32, 64] {
        put32(&mut f, v);
    }
    put16(&mut f, 1);
    put16(&mut f, 0xFFFF);
    put32(&mut f, 0);
    f.extend_from_slice(&id);
    put32(&mut f, 0); // flags, section flags
    put32(&mut f, 0); // qualifier
    put32(&mut f, 0); // offset
    put32(&mut f, section_len);
    f.extend_from_slice(&id);
    put32(&mut f, 0);
    put16(&mut f, 0);
    put16(&mut f, 0);
    put32(&mut f, section_len);
    put32(&mut f, 0);
    f.extend_from_slice(&b);
    put32(&mut f, 0xDEF5_FADE);
    put32(&mut f, section_len);
    put32(&mut f, 0xDEFF_FADE);
    put32(&mut f, size);
    f.extend_from_slice(b"mrm_pri2");
    f
}

/// The unit run "A", NUL, as a name block.
const ONE_NAME: [u8; 4] = [b'A', 0, 0, 0];

#[test]
fn crafted_schema_builder_is_valid() {
    let nodes = [(0, 0, 0, 0x10, 0, 0), (0, 1, 1, 0, 0, 0)];
    assert!(pri::read(&schema_file(&nodes, &ONE_NAME)).is_ok());
}

#[test]
fn many_names_into_one_unterminated_run_are_an_error() {
    // Thousands of nodes pointing into one long run of non-NUL units, with and without a
    // terminator at its very end.
    let mut nodes = vec![(0, 0, 0, 0x10, 0, 0)];
    nodes.extend((0..3000u16).map(|i| (0, 1, 0, 0, i, i)));
    for end in [*b"AA", [0, 0]] {
        let mut uni = [b'A', 0].repeat(0x4000 - 1);
        uni.extend_from_slice(&end);
        assert!(pri::read(&schema_file(&nodes, &uni)).is_err());
    }
}

#[test]
fn a_name_longer_than_its_node_says_is_an_error() {
    let nodes = [(0, 0, 0, 0x10, 0, 0), (0, 1, 2, 0, 0, 0)];
    assert!(pri::read(&schema_file(&nodes, &ONE_NAME)).is_err());
}

#[test]
fn a_parent_cycle_is_an_error() {
    let cycle = [
        (0, 0, 0, 0x10, 0, 0),
        (2, 1, 1, 0, 0, 0),
        (1, 1, 1, 0, 0, 1),
    ];
    let err = pri::read(&schema_file(&cycle, &ONE_NAME)).unwrap_err();
    assert!(err.expected.contains("cycle"), "{err}");
    let out_of_range = [(0, 0, 0, 0x10, 0, 0), (9, 1, 1, 0, 0, 0)];
    assert!(pri::read(&schema_file(&out_of_range, &ONE_NAME)).is_err());
}
