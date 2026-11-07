//! The writer: our own builds and, where the machine has them, Windows' PRIs (never committed)
//! come back byte for byte; hostile models don't panic it.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use pri::{Body, DataItem, Node, Pri, Schema, Scope, Section};

/// Where Windows keeps PRIs that makepri wrote, and how deep below them we look.
const WINDOWS_PRI_DIRS: [&str; 3] = [
    "/mnt/c/Windows/SystemApps",
    "/mnt/c/Windows/SystemResources",
    "/mnt/c/Windows/ImmersiveControlPanel",
];
const WINDOWS_PRI_DEPTH: usize = 3;
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

/// The `.pri` files under `dir`, at most `depth` folders down.
fn pris_under(dir: &Path, depth: usize, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            if depth > 0 {
                pris_under(&path, depth - 1, out);
            }
        } else if path
            .extension()
            .is_some_and(|e| e.eq_ignore_ascii_case("pri"))
        {
            out.push(path);
        }
    }
}

#[test]
fn windows_pris_round_trip() {
    let mut paths = Vec::new();
    for dir in WINDOWS_PRI_DIRS {
        pris_under(Path::new(dir), WINDOWS_PRI_DEPTH, &mut paths);
    }
    for path in paths {
        if !std::fs::metadata(&path).is_ok_and(|m| m.len() <= SYSTEM_PRI_LIMIT) {
            continue;
        }
        let data = std::fs::read(&path).unwrap();
        let name = path.display().to_string();
        let file = pri::read(&data).unwrap_or_else(|e| panic!("{name}: {e}"));
        let out = pri::write(&file).unwrap_or_else(|e| panic!("{name}: {e}"));
        assert!(out == data, "{name}: written bytes differ");
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

/// Makes `chain`'s schema an extended one, with an ASCII names pool, and returns it.
fn extended(file: &mut Pri) -> &mut Schema {
    file.sections[0].id = *b"[mrm_hschemaex] ";
    let Body::Schema(Some(h)) = &mut file.sections[0].body else {
        unreachable!()
    };
    h.extended = true;
    h.hnames_id = Some(*b"[def_hnamesx]  \0");
    h
}

fn read_schema(bytes: &[u8]) -> Schema {
    match pri::read(bytes).unwrap().sections.remove(0).body {
        Body::Schema(Some(h)) => h,
        _ => unreachable!(),
    }
}

#[test]
fn each_names_pool_with_names_opens_with_a_nul() {
    let mut file = chain(2);
    extended(&mut file).nodes[2].name = "ä".into();
    let h = read_schema(&pri::write(&file).unwrap());
    assert_eq!(h.names_ascii, b"\0a\0");
    assert_eq!(h.names_uni, [0, 0, 0xE4, 0, 0, 0]);
    assert!(!h.nodes[2].is_ascii);
    assert_eq!(h.nodes[2].offset, 1);
    assert_eq!(h.nodes[2].name, "ä");

    // No names at all: the UTF-16 pool holds the one NUL.
    let mut file = chain(0);
    extended(&mut file);
    let h = read_schema(&pri::write(&file).unwrap());
    assert!(h.names_ascii.is_empty());
    assert_eq!(h.names_uni, [0, 0]);
}

#[test]
fn the_names_pools_keep_bytes_past_their_names() {
    let mut file = chain(1);
    extended(&mut file).names_ascii = b"\0a\0\0\0\0".to_vec();
    let h = read_schema(&pri::write(&file).unwrap());
    assert_eq!(h.names_ascii, b"\0a\0\0\0\0");

    // Bytes that disagree with the names are not kept.
    let mut file = chain(1);
    extended(&mut file).names_ascii = b"\0b\0\0\0\0".to_vec();
    let h = read_schema(&pri::write(&file).unwrap());
    assert_eq!(h.names_ascii, b"\0a\0");
}

#[test]
fn blobs_start_on_8_bytes_of_the_body_and_follow_on_4() {
    // Header and tables take 12 + 4 + 2 * 8 = 32 bytes, so the string "a" leaves the data at
    // 36 of the body, and the first blob goes to 40: offset 8 in the data.
    let mut file = chain(0);
    file.sections[0].id = *b"[mrm_dataitem] \0";
    file.sections[0].body = Body::Data(DataItem {
        u0: 0,
        strs: vec![(0, 1)],
        blobs: vec![(1, 1), (2, 1)],
        data: b"abc".to_vec(),
        pad: Vec::new(),
    });
    let back = pri::read(&pri::write(&file).unwrap()).unwrap();
    let Body::Data(d) = &back.sections[0].body else {
        unreachable!()
    };
    assert_eq!(d.strs, [(0, 1)]);
    assert_eq!(d.blobs, [(8, 1), (12, 1)]);
    assert_eq!(d.values(), [b"a", b"b", b"c"]);
    assert_eq!((32 + d.data.len()) % 8, 0);
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
