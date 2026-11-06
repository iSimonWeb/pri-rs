# The PRI format, as measured

What we learned writing `resources.pri` files that Windows accepts, checked against the PRIs
under `C:\Windows\SystemApps` (makepri's own output) and against MRT Core's source. Where
PriTools' notes differ, these are right. Integers are little-endian.

This covers what small packages hold: one `Files` resource map of file candidates. Strings,
blobs, merged PRIs and resource packs are outside it (see the README).

## 1. File

- Header (32 bytes): `mrm_pri2`, u16 0, u16 1, u32 file size, u32 TOC offset (32), u32 first
  section's offset, u16 section count, u16 0xFFFF, u32 0.
- TOC: one 32-byte entry per section: 16-byte identifier, u16 flags, u16 section flags, u32
  qualifier (all 0 in small packages), u32 offset from the first section, u32 length.
- Footer (16 bytes): u32 `0xDEFFFADE`, u32 file size, `mrm_pri2`. PriTools' notes print both
  footer magics byte-swapped.
- Each section: a 32-byte header (identifier, u32 qualifier, u16 flags, u16 section flags,
  u32 length, u32 0), the body padded to 8 bytes, and a footer (u32 `0xDEF5FADE`, u32
  length). The length counts header, padded body and footer.
- A small package's sections, in this order: `[mrm_decn_info]`, `[mrm_pridescex]`,
  `[mrm_hschemaex] `, `[mrm_res_map2_]`, then one or more `[mrm_dataitem] `.

## 2. Descriptor (`[mrm_pridescex]`)

Flags 1 (AutoMerge), included file list 0xFFFF, 0, then the counts of schemas, decision
infos, resource maps, the primary map's index, the counts of file lists and data items, 0, and
the section indices of each kind in that order.

## 3. Names (`[mrm_hschemaex] `, `[def_hnamesx]`)

- Unique name `ms-appx://<Identity Name>/`, name `<Identity Name>`, version 1.0.
- Nodes: the root (empty name), then scopes and items. A scope's children are contiguous and
  sorted case-insensitively, and are laid out when the scope is visited, depth first
  (root's children, then the first child scope's children, and so on). Each node record holds
  the parent's *node* index, the full path's length (`Files/Assets/X.png`), the name's
  uppercased first character, its length, flags (0x10 scope, 0x20 ASCII) and its offset.
- Name blocks: the ASCII block starts with a NUL, then scope names by scope index, then item
  names by item index, each NUL-terminated. Names that aren't ASCII go in the UTF-16 block.
- The names' data is padded to its own 8-byte total (the `total` field), and then the section
  body is padded as every section's is. The writer applies both in that order, and that
  matches every file we compared.
- Checksum: plain CRC-32 (zlib's), chained, as MRT Core's
  `ComputeHierarchicalSchemaVersionChecksum`: the unique name, the name, `major | minor << 16`,
  then for scopes and then items: a null description (8 zero bytes), u32 1
  (case-insensitive), u32 count, and each full name (`Files/Assets/X.png`, `/`-separated,
  ASCII-lowercased). Each string is hashed as its byte length with the terminator (u32), then
  its UTF-16 bytes with the terminator. It matched every file.

## 4. Qualifiers (`[mrm_decn_info]`)

- Distinct qualifiers: (u16 2, u16 type, u16 0, u16 10, u32 value offset), values UPPERCASE in a
  UTF-16 block. Every file opens with an empty Language one, (0, 0, 0, 1, 0), and a block
  starting with a NUL.
- Qualifiers: (distinct index, priority, fallback score, 0). Priorities: Contrast 400, Theme
  350, TargetSize 300, Scale 200, AlternateForm 100.
- Fallback scores: TargetSize 256 → 1000, any other 500; AlternateForm 0; Contrast STANDARD
  1000, others 0; Theme DARK 1000, LIGHT 0; Scale by distance from the default 100: 80 → 650,
  100 → 1000, 125 → 937, 140 → 900, 150 → 875, 180 → 800, 200 → 750, 400 → 437.
- Qualifier sets list their qualifiers by priority, highest first. Set 0 is neutral (empty).
  Decision 0 is empty, decision 1 is the neutral set alone.
- An item's candidates are sorted by qualifier, highest priority first. Within one qualifier,
  values sort in descending string order ("96" before "80" before "256" before "24"), and an
  absent qualifier comes after every value. Example: targetsize-96 unplated, lightunplated,
  plain, then 80, …, then the scales, the neutral last.
- The index table holds the sets' qualifier lists and the decisions' set lists. makepri
  reuses runs it already has; any layout reads the same.

## 5. Resource map (`[mrm_res_map2_]`) and data (`[mrm_dataitem] `)

- Map: no environment references, no schema reference, the seven value types `(4, i)`, one
  item-to-group entry (first item 0, group 0), one group (item count, 0), then per item
  (decision, first candidate), and per candidate `01 05 0000 <data item index> <section>`:
  type 1 (a data item), value type 5 (AsciiPath).
- Data items: strings, each at a 4-byte boundary, the total padded so the body ends on 8
  bytes. Values are paths with backslashes and a NUL: `Assets\X.targetsize-24_altform-unplated.png`.
  One section for every candidate works; makepri sometimes splits them.

## 6. What packages index

Most of Windows' packages index only their assets, not their exe or DLLs. Most also add a
`MicrosoftInternalMetadata/ContentChecksumValue` string item; `pri new` leaves it out, and
Windows doesn't miss it.

makepri names folders as they are on disk, so Windows' own packages say `assets` where ours
say `Assets`. Lookups are case-insensitive.

## 7. Not yet modelled

- **Blobs** (compiled XAML) in data items. Their alignment depends on something we haven't
  found: putting each blob at an 8-byte boundary of the section body fixes some files and
  breaks others. Files with blobs read, but don't round-trip byte for byte.
- **String resources** (`Resources/` maps, `lang-*` qualifiers), `[def_file_list]`, and the
  u32 extension tables that larger files use.
