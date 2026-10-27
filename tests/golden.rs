//! The builder against the goldens: PRIs we made ourselves from asset name lists (empty files,
//! names only). Microsoft's own are not kept here.

use std::path::{Path, PathBuf};
use std::process::Command;

const UDC_NAME: &str = "MadebySimon.UltimateDesktopCleaner";

fn golden(file: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/golden")
        .join(file)
}

/// The asset names of a list, as layout-relative paths.
fn assets(stem: &str) -> Vec<String> {
    std::fs::read_to_string(golden(&format!("{stem}-assets.txt")))
        .unwrap()
        .lines()
        .filter(|l| !l.is_empty())
        .map(|l| format!("Assets/{l}"))
        .collect()
}

fn check(stem: &str, name: &str) {
    let mut files = assets(stem);
    files.sort();
    let built = pri::build(name, &files).unwrap();
    let bytes = pri::write(&built).unwrap();
    let want = std::fs::read(golden(&format!("{stem}.pri"))).unwrap();
    assert!(
        bytes == want,
        "{stem}: built {} bytes, golden {}",
        bytes.len(),
        want.len()
    );
}

#[test]
fn udc() {
    check("udc", UDC_NAME);
}

#[test]
fn parental_controls() {
    let stem = "ParentalControls_cw5n1h2txyewy";
    check(stem, &format!("Test.{stem}"));
}

#[test]
fn add_suggested_folders() {
    let stem = "Microsoft.Windows.AddSuggestedFoldersToLibraryDialog_cw5n1h2txyewy";
    check(stem, &format!("Test.{stem}"));
}

#[test]
fn new_on_a_layout() {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("layout");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("Assets")).unwrap();
    for f in assets("udc") {
        std::fs::write(dir.join(&f), b"").unwrap();
    }
    std::fs::write(
        dir.join("AppxManifest.xml"),
        format!("<Package>\n  <Identity Version=\"1.0.0.0\" Name='{UDC_NAME}'/>\n</Package>"),
    )
    .unwrap();
    let run = || {
        Command::new(env!("CARGO_BIN_EXE_pri"))
            .arg("new")
            .arg(&dir)
            .status()
            .unwrap()
    };
    // A second run finds its own output beside the assets and leaves it out.
    for _ in 0..2 {
        assert!(run().success());
        let got = std::fs::read(dir.join("resources.pri")).unwrap();
        assert!(got == std::fs::read(golden("udc.pri")).unwrap());
    }
}
