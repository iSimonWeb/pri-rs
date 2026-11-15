//! The writer: our own builds and, where the machine has them, Windows' PRIs (never committed)
//! come back byte for byte; hostile models don't panic it.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use pri::{
    Body, Candidate, DataItem, FileEntry, FileList, Folder, LargeFrom, Node, Pri, ResMap, Schema,
    Scope, Section,
};

/// Where Windows keeps PRIs that makepri wrote, and the ones it merges itself (`rescache`), and
/// how deep below them we look.
const WINDOWS_PRI_DIRS: [&str; 4] = [
    "/mnt/c/Windows/SystemApps",
    "/mnt/c/Windows/SystemResources",
    "/mnt/c/Windows/ImmersiveControlPanel",
    "/mnt/c/Windows/rescache",
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
            reserved: 0,
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

/// A map of `n` items in one group, each with one candidate.
fn map_file(n: u32, large_from: Option<LargeFrom>) -> Pri {
    let mut file = chain(0);
    file.sections[0].id = *b"[mrm_res_map2_]\0";
    file.sections[0].body = Body::Map(ResMap {
        v2: true,
        env: Vec::new(),
        env_count: 0,
        schema_section: 0,
        schema_ref: Vec::new(),
        decn_section: 0,
        types: vec![(4, 0)],
        flags: 0,
        item_to_group: vec![(0, 0)],
        groups: vec![(n, 0)],
        infos: (0..n).map(|i| (1, i)).collect(),
        large_from,
        candidates: (0..n)
            .map(|_| Candidate::Embedded {
                value_type: 0,
                reserved: 0,
                value: 0,
            })
            .collect(),
        data: Vec::new(),
        pad: Vec::new(),
    });
    file
}

#[test]
fn a_map_past_16_bits_moves_its_tables_to_the_large_block() {
    // 0x10002 items of one candidate each: too many for the 16-bit tables alone.
    let n = 0x1_0002;
    let mut file = map_file(n, None);
    assert!(pri::write(&file).is_err());

    let Body::Map(m) = &mut file.sections[0].body else {
        unreachable!()
    };
    // makepri's split: item 0xFFFF's candidates end past 0xFFFF, so it and the rest go large,
    // in a range of their own.
    m.item_to_group = vec![(0, 0), (0xFFFF, 1)];
    m.groups = vec![(0xFFFF, 0), (3, 0xFFFF)];
    m.large_from = Some(LargeFrom {
        item_to_group: 1,
        groups: 1,
        infos: 0xFFFF,
    });
    let bytes = pri::write(&file).unwrap();
    let back = pri::read(&bytes).unwrap();
    assert_eq!(back.sections[0].body, file.sections[0].body);
    assert_eq!(pri::write(&back).unwrap(), bytes);
}

#[test]
fn a_large_block_that_starts_inside_no_table_is_an_error() {
    for (item_to_group, groups, infos) in [(2, 1, 1), (1, 2, 1), (1, 1, 3)] {
        let file = map_file(
            2,
            Some(LargeFrom {
                item_to_group,
                groups,
                infos,
            }),
        );
        assert!(
            pri::write(&file).is_err(),
            "{item_to_group} {groups} {infos}"
        );
    }
}

/// A two-item map whose second info sits in the large block, and where in the bytes the map's
/// body and its large block start.
fn map_with_large_block() -> (Vec<u8>, usize, usize) {
    let file = map_file(
        2,
        Some(LargeFrom {
            item_to_group: 1,
            groups: 1,
            infos: 1,
        }),
    );
    let bytes = pri::write(&file).unwrap();
    let id = b"[mrm_res_map2_]\0";
    let section = bytes.windows(16).rposition(|w| w == id).unwrap();
    let body = section + 32;
    // Header, one value type, then one 4-byte entry in each 16-bit table.
    let large = body + 32 + 8 + 3 * 4;
    assert_eq!(bytes[body + 28], 20, "the large block's length");
    assert_eq!(
        bytes[large..large + 12],
        [0, 0, 0, 0, 0, 0, 0, 0, 1, 0, 0, 0]
    );
    (bytes, body, large)
}

#[test]
fn a_malformed_large_block_is_an_error() {
    let (bytes, body, large) = map_with_large_block();
    assert!(pri::read(&bytes).is_ok());
    let broken: [(&str, usize, u8); 4] = [
        ("flags asking for 10-byte candidates", body + 18, 1),
        ("a block shorter than its counts", body + 28, 4),
        ("counts that don't fill the block", large, 1),
        ("an info's pad that isn't 0", large + 12 + 2, 1),
    ];
    for (what, at, value) in broken {
        let mut b = bytes.clone();
        b[at] = value;
        assert!(pri::read(&b).is_err(), "{what}");
    }
}

#[test]
fn writing_what_hostile_large_blocks_parse_to_never_panics() {
    let (bytes, body, _) = map_with_large_block();
    for i in body..bytes.len() {
        for flip in [0x01, 0x80, 0xFF] {
            let mut b = bytes.clone();
            b[i] ^= flip;
            if let Ok(file) = pri::read(&b) {
                let _ = file.listing();
                let _ = pri::write(&file);
            }
        }
    }
}

fn folder(parent: i16, subfolders: (u16, i16), files: (u16, i16), name: &str) -> Folder {
    Folder {
        flags: 0,
        parent,
        subfolders: subfolders.0,
        first_subfolder: subfolders.1,
        files: files.0,
        first_file: files.1,
        name: name.into(),
    }
}

fn file(parent: i16, name: &str) -> FileEntry {
    FileEntry {
        flags: 2,
        parent,
        name: name.into(),
    }
}

/// `C:\App\resources.pri` and `C:\App\pris\Resources.it-IT.pri` and `...\RESOURCES.PRI`.
fn file_list() -> FileList {
    FileList {
        roots: 1,
        longest_path: 0,
        folders: vec![
            folder(-1, (1, 1), (0, -1), "C:"),
            folder(0, (1, 2), (1, 0), "App"),
            folder(1, (0, -1), (2, 1), "pris"),
        ],
        files: vec![
            file(1, "resources.pri"),
            file(2, "Resources.it-IT.pri"),
            file(2, "RESOURCES.PRI"),
        ],
        names: Vec::new(),
        pad: Vec::new(),
    }
}

fn with_body(id: &[u8; 16], body: Body) -> Pri {
    let mut pri = chain(0);
    pri.sections[0].id = *id;
    pri.sections[0].body = body;
    pri
}

#[test]
fn a_file_list_lays_out_its_names_once_each_ignoring_case() {
    let mut list = file_list();
    // The body is 12 + 3 * 20 + 3 * 12 bytes before the pool; 4 more end it on 8.
    list.pad = vec![0; 4];
    let pri = with_body(b"[def_file_list]\0", Body::FileList(list));
    let back = pri::read(&pri::write(&pri).unwrap()).unwrap();
    let Body::FileList(f) = &back.sections[0].body else {
        unreachable!()
    };
    let pool: Vec<u16> = "\0C:\0App\0resources.pri\0pris\0Resources.it-IT.pri\0"
        .encode_utf16()
        .collect();
    let bytes: Vec<u8> = pool.iter().flat_map(|u| u.to_le_bytes()).collect();
    assert_eq!(f.names, bytes);
    // The third file shares the first's name, as stored.
    assert_eq!(f.files[2].name, "resources.pri");
    let paths: Vec<_> = (0..3).filter_map(|i| f.file_path(i)).collect();
    assert_eq!(
        paths,
        [
            "C:\\App\\resources.pri",
            "C:\\App\\pris\\Resources.it-IT.pri",
            "C:\\App\\pris\\resources.pri"
        ]
    );
}

#[test]
fn names_that_fold_to_another_length_are_not_shared() {
    let mut list = file_list();
    list.files[1].name = "SS".into();
    list.files[2].name = "ß".into();
    let pri = with_body(b"[def_file_list]\0", Body::FileList(list));
    let back = pri::read(&pri::write(&pri).unwrap()).unwrap();
    let Body::FileList(f) = &back.sections[0].body else {
        unreachable!()
    };
    assert_eq!(f.files[1].name, "SS");
    assert_eq!(f.files[2].name, "ß");
}

#[test]
fn a_path_whose_parents_loop_or_dangle_starts_with_a_question_mark() {
    let mut list = file_list();
    list.folders[0].parent = 2;
    assert!(list.file_path(0).unwrap().starts_with("?\\"));
    list.folders[0].parent = 7;
    assert_eq!(list.file_path(0).unwrap(), "?\\C:\\App\\resources.pri");
    assert_eq!(list.file_path(3), None);
}

#[test]
fn overlapping_subfolder_ranges_fail_fast() {
    // Folder i claims every folder after it: a tree only once, and quadratic if not caught.
    let n: u16 = 20_000;
    let folders = (0..n)
        .map(|i| {
            let rest = n - 1 - i;
            let first = if rest == 0 { -1 } else { i as i16 + 1 };
            folder(i as i16 - 1, (rest, first), (0, -1), "a")
        })
        .collect();
    let list = FileList {
        roots: 1,
        longest_path: 0,
        folders,
        files: Vec::new(),
        names: Vec::new(),
        pad: Vec::new(),
    };
    let pri = with_body(b"[def_file_list]\0", Body::FileList(list));
    let start = Instant::now();
    assert!(pri::write(&pri).is_err());
    assert!(start.elapsed() < DEEP_CHAIN_BUDGET);
}

/// A written file-list PRI, with where its section body starts.
fn file_list_bytes() -> (Vec<u8>, usize) {
    let mut list = file_list();
    list.pad = vec![0; 4];
    let bytes = pri::write(&with_body(b"[def_file_list]\0", Body::FileList(list))).unwrap();
    // The TOC names the section first; its own header comes later.
    let section = bytes
        .windows(16)
        .rposition(|w| w == b"[def_file_list]\0")
        .unwrap();
    (bytes, section + 32)
}

#[test]
fn a_file_list_name_outside_its_pool_is_an_error() {
    let (bytes, body) = file_list_bytes();
    assert!(pri::read(&bytes).is_ok());
    // The first file's name offset: after the header and three folders, 8 bytes in.
    let at = body + 12 + 3 * 20 + 8;
    let mut b = bytes.clone();
    b[at + 2] = 0x7F;
    assert!(pri::read(&b).is_err());
    // A lone surrogate in the pool.
    let pool = body + 12 + 3 * 20 + 3 * 12;
    let mut b = bytes.clone();
    b[pool + 2..pool + 4].copy_from_slice(&0xD800u16.to_le_bytes());
    assert!(pri::read(&b).is_err());
}

#[test]
fn writing_what_hostile_file_lists_parse_to_never_panics() {
    let (bytes, body) = file_list_bytes();
    for i in body..bytes.len() {
        for flip in [0x01, 0x80, 0xFF] {
            let mut b = bytes.clone();
            b[i] ^= flip;
            if let Ok(file) = pri::read(&b) {
                if let Body::FileList(f) = &file.sections[0].body {
                    (0..f.files.len()).for_each(|j| drop(f.file_path(j)));
                }
                let _ = file.listing();
                let _ = pri::write(&file);
            }
        }
    }
}

#[test]
fn the_older_and_newer_magics_read_and_others_do_not() {
    for magic in ["mrm_pri1", "mrm_pri3"] {
        let mut pri = chain(1);
        pri.magic = magic.into();
        let bytes = pri::write(&pri).unwrap();
        assert_eq!(pri::read(&bytes).unwrap().magic, magic);
    }
    let mut pri = chain(1);
    pri.magic = "mrm_pri9".into();
    assert!(pri::read(&pri::write(&pri).unwrap()).is_err());
}

#[test]
fn a_file_list_whose_folders_do_not_form_a_tree_is_an_error() {
    let mut looped = file_list();
    looped.folders[2].subfolders = 1;
    looped.folders[2].first_subfolder = 1;
    let mut unreached = file_list();
    unreached.folders[1].subfolders = 0;
    let mut outside = file_list();
    outside.folders[1].first_file = 2;
    for (what, list) in [
        ("a loop", looped),
        ("an unreached folder", unreached),
        ("files past the end", outside),
    ] {
        let pri = with_body(b"[def_file_list]\0", Body::FileList(list));
        assert!(pri::write(&pri).is_err(), "{what}");
    }
}

#[test]
fn bytes_reserved_after_a_section_come_back() {
    let mut pri = chain(1);
    pri.sections[0].reserved = 24;
    let bytes = pri::write(&pri).unwrap();
    let back = pri::read(&bytes).unwrap();
    assert_eq!(back.sections[0].reserved, 24);
    assert_eq!(pri::write(&back).unwrap(), bytes);

    // A gap that isn't zeros and a footer counting it is not reserved, and the writer drops it.
    let gap_end = bytes.len() - 16;
    for (what, at, value) in [
        ("a byte that isn't 0", gap_end - 9, 1),
        ("a footer that doesn't count the gap", gap_end - 4, 0),
    ] {
        let mut b = bytes.clone();
        b[at] = value;
        assert_eq!(pri::read(&b).unwrap().sections[0].reserved, 0, "{what}");
    }

    pri.sections[0].reserved = 4;
    assert!(pri::write(&pri).is_err());
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
