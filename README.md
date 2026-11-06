# pri-rs

Reads, writes and builds Windows `resources.pri` files, on any platform. It is a Rust library
and a CLI, both named `pri`, with no dependencies beyond `std`.

## Why

Windows finds a package's resource variants only through its `resources.pri`. That includes
the `altform-unplated` logos, and without them the taskbar, Start and Alt+Tab draw the icon on
an accent-coloured plate. Microsoft's `makepri.exe` writes that file, but only on Windows.
`makemsix`, which packs MSIX on Linux, never indexes resources, so a package packed there has
no PRI and gets the plate.

`pri new` writes the PRI from the package's asset file names, so an MSIX can be packed
entirely on Linux, CI runners included. It builds the Microsoft Store packages of
Ultimate Desktop Cleaner, whose logos show without the plate.

## Install

```sh
cargo install --git https://github.com/iSimonWeb/pri-rs
```

It isn't on crates.io yet.

## CLI

```sh
pri new <layout> [--name <Identity Name>] [--out <file>] [--index <dir>]...
pri dump <file.pri>
pri roundtrip <file.pri>...
```

- `new` indexes the layout's `Assets` folder (or each `--index` folder) and writes
  `<layout>/resources.pri`. Without `--name`, it reads the `Identity Name` from
  `<layout>/AppxManifest.xml`. Qualifiers come from the file names (`scale-200`,
  `targetsize-24_altform-unplated`, `theme-dark`, `contrast-high` and so on), with the same
  priorities, fallback scores and candidate order that makepri gives them.
- `dump` prints every section, the names with their full paths, each item's candidates as
  qualifier strings, and the schema checksum computed against the stored one.
- `roundtrip` reads each file, writes it back and compares the bytes.

A typical packing step, before `makemsix pack`:

```sh
pri new build/layout            # writes build/layout/resources.pri
pri dump build/layout/resources.pri
```

## Library

```rust
let pri = pri::read(&std::fs::read("resources.pri")?)?;   // bytes -> model
let bytes = pri::write(&pri)?;                              // model -> bytes

let layout = std::path::Path::new("build/layout");
let files = pri::scan(layout, &[pri::DEFAULT_INDEX.to_string()], None)?;
let built = pri::build("Contoso.App", &files)?;             // asset paths -> model
```

`build_layout` does all of `pri new` in one call. Malformed input is an error, never a panic:
every read stays inside its block, and parent cycles are rejected.

## What it covers

- **Reads** every section that small packages hold: `[mrm_decn_info]`, `[mrm_pridescex]`,
  `[mrm_hschemaex] `, `[mrm_res_map2_]` and `[mrm_dataitem] `.
- **Writes** a model back with every length, offset, padding and checksum recomputed. Files
  without blobs come back byte for byte.
- **Builds** a `Files` resource map of file candidates under `scale`, `targetsize`, `altform`,
  `contrast` and `theme`.

Not yet covered:

- `lang-*` file qualifiers in `pri new`.
- Blobs (compiled XAML): they read, but don't round-trip byte for byte.
- String resources (`Resources/` maps), `[def_file_list]`, merged PRIs and resource
  packs, and the u32 extension tables that larger files use.

[docs/format.md](docs/format.md) describes the format as we measured it, including what is
still unknown.

## Tests

```sh
cargo test --release
```

The goldens in `tests/golden/` are PRIs that we built ourselves: ours, and two built from the
asset name lists of Windows packages (names only, no content). We never commit Microsoft's
PRIs. When `/mnt/c/Windows/SystemApps` is present (WSL), one test also round-trips the PRIs
it finds there, and it skips when the folder is absent.

## Credits

Two projects taught us the format. We read them, and the code here is our own:

- [MRT Core](https://github.com/microsoft/WindowsAppSDK/tree/main/dev/MRTCore) in the
  Windows App SDK (MIT), Microsoft's own resource code. It settled the schema checksum and the
  builder's layout.
- [PriTools](https://github.com/chausner/PriTools) (Apache-2.0), whose
  reverse-engineered format notes were where we started.

## License

MIT. See [LICENSE](LICENSE).
