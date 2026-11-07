# Changelog

All notable changes to this project are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and versions follow `major.minor.fix`.

## [Unreleased]

## [0.4.0] - 2026-11-07

### Changed

- The resource map's large block is decoded: the directory, groups and items continue there
  in 32 bits from the first item whose index, or whose candidates' end, passes 0xFFFF, as
  MRT Core's `MapBuilder` lays them out. `ResMap`'s three tables now hold both parts in 32-bit fields, `large_from` says
  where the large part starts, and the header's `flags` has a field of its own (it had been
  read as the high half of the item count). `pri dump` used to stop at the 16-bit items: on
  `MicrosoftWindows.Client.CBS` it listed 691 of 3776 items, and now lists all of them.
  All 487 `mrm_pri2` files on our Windows machine still come back identical, the 4 with a
  large block included. A map whose flags ask for 10-byte candidates is an error, since no
  file we have uses them, and so is a large block with bytes its counts don't account for,
  which MRT Core would ignore.

## [0.3.2] - 2026-11-07

### Fixed

- Blobs (compiled XAML) round-trip byte for byte. The first blob starts at an 8-byte boundary
  of the section body and each one after it at 4, as MRT Core's `DataItemsSectionBuilder` lays
  out its large items. In 0.2.0 we had put every blob on 8, which fixed some files and broke
  others. Measured on the 45 files with blobs, the only gaps beyond 4-byte rounding came at the
  first blob, 128 of them, each 4 bytes. All 363 PRIs under `C:\Windows`'s `SystemApps`,
  `ImmersiveControlPanel` and `SystemResources` now come back identical, and the WSL test
  round-trips all three folders instead of `SystemApps` alone, with no exception for blobs.

## [0.3.1] - 2026-11-07

### Fixed

- The writer lays out the names pools as MRT Core's HNamesBuilder does: each pool that holds
  names starts with a NUL, and when neither does, the UTF-16 one holds that NUL. We had
  seeded only the ASCII pool. A pool that makepri sized larger than its names keeps its extra
  bytes. Of the 363 PRIs under `C:\Windows`'s `SystemApps`, `ImmersiveControlPanel` and
  `SystemResources`, 336 now come back identical, up from 333. The three fixed are the two
  root-only `Microsoft.Windows.Private.CacheManagement.pri` (a `[def_hnames]` schema) and
  `Windows.Activation.pri`, whose ASCII pool ends in 21 zero bytes that no name uses. The
  other 27 all hold blobs, and so do 18 of the identical files.
- The same census shows that what the README listed as gaps already reads and round-trips:
  310 of the identical files hold `Resources/` string maps, 265 of the 272 resource packs
  (`lang-*` qualifiers) are identical, and the 4 files with the resource map's large tables
  differ only in their blobs. None of the 363 holds a `[def_file_list]` or a merged
  descriptor.

### Added

- A repository of its own, with the history it grew in Ultimate Desktop Cleaner's
  `src-pri`, where it builds the `resources.pri` of every Store package since that app's
  0.40.2. MIT licence, a README with the CLI and what the tool covers, and
  [docs/format.md](docs/format.md), the format as we measured it. Versions 0.1.0 to 0.3.0
  below are numbered retroactively, one per commit of that history.

## [0.3.0] - 2026-10-27

### Added

- `pri new <layout>` builds a package's `resources.pri` from its asset file names: qualifiers
  read from the names (`scale`, `targetsize`, `altform`, `contrast`, `theme`), with the
  priorities, fallback scores and candidate order that makepri gives them. Rebuilt from their
  asset name lists alone, ParentalControls' `FamilySafetyAppList.png` (44 candidates) and
  AddSuggestedFolders' `Square44x44Logo.png` match makepri's files candidate for candidate.
  Goldens for those two lists and for our own layout pin the output byte for byte.

## [0.2.0] - 2026-10-27

### Added

- The writer turns a model back into bytes, recomputing every length, offset, padding and
  checksum, and `pri roundtrip` checks a file byte for byte. Of the 48 files in our set (the
  `resources.pri` of each package under `C:\Windows\SystemApps`, with copies), 41 come back
  identical. The other 7 hold blobs of compiled XAML, whose alignment we haven't modelled:
  putting each blob at an 8-byte boundary of the section body fixed some files and broke
  others (39 of 48 identical instead of 41), so we dropped it.

## [0.1.0] - 2026-10-27

### Added

- The reader parses a `resources.pri` (`mrm_pri2`) into a typed model that keeps everything a
  writer needs, and `pri dump` prints its sections, names, each item's candidates, and the
  schema checksum computed against the stored one. It reads every `resources.pri` under
  `SystemApps` with matching checksums. Malformed input is an error, never a panic.

[Unreleased]: https://github.com/iSimonWeb/pri-rs/compare/v0.4.0...HEAD
[0.4.0]: https://github.com/iSimonWeb/pri-rs/compare/v0.3.2...v0.4.0
[0.3.2]: https://github.com/iSimonWeb/pri-rs/compare/v0.3.1...v0.3.2
[0.3.1]: https://github.com/iSimonWeb/pri-rs/compare/v0.3.0...v0.3.1
[0.3.0]: https://github.com/iSimonWeb/pri-rs/compare/v0.2.0...v0.3.0
[0.2.0]: https://github.com/iSimonWeb/pri-rs/compare/v0.1.0...v0.2.0
[0.1.0]: https://github.com/iSimonWeb/pri-rs/releases/tag/v0.1.0
