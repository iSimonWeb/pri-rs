# Changelog

All notable changes to this project are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and versions follow `major.minor.fix`.

## [Unreleased]

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

[Unreleased]: https://github.com/iSimonWeb/pri-rs/compare/v0.3.0...HEAD
[0.3.0]: https://github.com/iSimonWeb/pri-rs/compare/v0.2.0...v0.3.0
[0.2.0]: https://github.com/iSimonWeb/pri-rs/compare/v0.1.0...v0.2.0
[0.1.0]: https://github.com/iSimonWeb/pri-rs/releases/tag/v0.1.0
