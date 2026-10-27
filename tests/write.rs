//! The writer: our own builds and, where the machine has them, Windows' PRIs (never committed)
//! come back byte for byte; hostile models don't panic it.

use std::path::Path;
use std::time::{Duration, Instant};

use pri::{Body, Node, Pri, Schema, Scope, Section};

/// Windows' PRIs above this size are skipped, to keep the test quick.
const SYSTEM_PRI_LIMIT: u64 = 3 * 1024 * 1024;
/// A chain this deep must checksum in well under this.
const DEEP_CHAIN_LEVELS: usize = 5000;
const DEEP_CHAIN_BUDGET: Duration = Duration::from_secs(5);

fn golden_bytes(name: &str) -> Vec<u8> {
    std::fs::read(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/golden")
            .join(name),
    )
    .unwrap()
}

fn assert_identical(name: &str, data: &[u8]) {
    let file = pri::read(data).unwrap_or_else(|e| panic!("{name}: {e}"));
    let out = pri::write(&file).unwrap_or_else(|e| panic!("{name}: {e}"));
    assert!(out == data, "{name}: written bytes differ");
}

#[test]
fn goldens_round_trip() {
    for name in [
        "udc.pri",
        "ParentalControls_cw5n1h2txyewy.pri",
        "Microsoft.Windows.AddSuggestedFoldersToLibraryDialog_cw5n1h2txyewy.pri",
    ] {
        assert_identical(name, &golden_bytes(name));
    }
}

fn holds_blobs(file: &Pri) -> bool {
    file.sections
        .iter()
        .any(|s| matches!(&s.body, Body::Data(d) if !d.blobs.is_empty()))
}

#[test]
fn system_apps_round_trip_unless_they_hold_blobs() {
    let Ok(dir) = std::fs::read_dir("/mnt/c/Windows/SystemApps") else {
        return;
    };
    for entry in dir.flatten() {
        let path = entry.path().join("resources.pri");
        if !std::fs::metadata(&path).is_ok_and(|m| m.len() <= SYSTEM_PRI_LIMIT) {
            continue;
        }
        let data = std::fs::read(&path).unwrap();
        let name = path.display().to_string();
        let file = pri::read(&data).unwrap_or_else(|e| panic!("{name}: {e}"));
        let out = pri::write(&file).unwrap_or_else(|e| panic!("{name}: {e}"));
        // Blobs are laid out as we choose, but what we write must read back the same.
        if holds_blobs(&file) {
            let again = pri::read(&out).unwrap_or_else(|e| panic!("{name}: {e}"));
            assert_eq!(pri::write(&again).unwrap(), out, "{name}");
        } else {
            assert!(out == data, "{name}: written bytes differ");
        }
    }
}

/// A compact schema of one root scope and `levels` items, each the child of the one before.
fn chain(levels: usize) -> Pri {
    let mut nodes = vec![Node {
        parent: 0,
        full_len: 0,
        first: 0,
        name_len: 0,
        is_scope: true,
        is_ascii: false,
        flags_hi: 0,
        offset: 0,
        index: 0,
        name: String::new(),
    }];
    for i in 1..=levels {
        nodes.push(Node {
            parent: (i - 1) as u16,
            full_len: 0,
            first: 0,
            name_len: 0,
            is_scope: false,
            is_ascii: false,
            flags_hi: 0,
            offset: 0,
            index: (i - 1) as u16,
            name: "a".into(),
        });
    }
    let schema = Schema {
        extended: false,
        hnames_id: None,
        major: 1,
        minor: 0,
        checksum: 0,
        unique_name: "u".into(),
        name: "n".into(),
        max_full_path: 0,
        total: 0,
        nodes,
        scopes: vec![Scope {
            node: 0,
            children: levels as u16,
            first_child: 1,
        }],
        items: (1..=levels as u16).collect(),
        names_uni: Vec::new(),
        names_ascii: Vec::new(),
        pad: Vec::new(),
    };
    Pri {
        magic: "mrm_pri2".into(),
        header_u8: 0,
        header_u10: 1,
        header_u26: 0xFFFF,
        header_u28: 0,
        sections: vec![Section {
            id: *b"[mrm_hschema]\0\0\0",
            toc_flags: 0,
            toc_section_flags: 0,
            toc_qualifier: 0,
            header_qualifier: 0,
            header_flags: 0,
            header_section_flags: 0,
            header_u28: 0,
            body: Body::Schema(Some(schema)),
        }],
    }
}

#[test]
fn a_deep_parent_chain_checksums_quickly() {
    let file = chain(DEEP_CHAIN_LEVELS);
    let Body::Schema(Some(h)) = &file.sections[0].body else {
        unreachable!()
    };
    let start = Instant::now();
    let checksum = h.checksum();
    let bytes = pri::write(&file).unwrap();
    assert!(start.elapsed() < DEEP_CHAIN_BUDGET, "{:?}", start.elapsed());

    let names = h.full_names();
    assert_eq!(
        names[DEEP_CHAIN_LEVELS].split('/').count(),
        DEEP_CHAIN_LEVELS
    );
    assert_eq!(names[3], h.full_name(3));

    let back = pri::read(&bytes).unwrap();
    let Body::Schema(Some(h)) = &back.sections[0].body else {
        unreachable!()
    };
    assert_eq!(h.checksum, checksum);
    assert_eq!(h.checksum(), checksum);
}

#[test]
fn a_model_that_does_not_fit_is_an_error_not_a_panic() {
    let mut file = chain(2);
    let Body::Schema(Some(h)) = &mut file.sections[0].body else {
        unreachable!()
    };
    h.unique_name = "x".repeat(70_000);
    assert!(pri::write(&file).is_err());
    let mut file = chain(2);
    file.magic = "short".into();
    assert!(pri::write(&file).is_err());
}

#[test]
fn writing_what_hostile_files_parse_to_never_panics() {
    let data = golden_bytes("udc.pri");
    for i in 0..data.len() {
        let mut broken = data.clone();
        broken[i] ^= 0xFF;
        if let Ok(file) = pri::read(&broken) {
            let _ = pri::write(&file);
        }
    }
}
