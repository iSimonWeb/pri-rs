//! A package layout's asset file names to the model `write` serialises: qualifiers read from the
//! names, the resource names, qualifier sets, decisions and candidates. Only the fields the
//! writer copies are set to what makepri writes; it computes the rest.

use std::collections::BTreeMap;
use std::fmt;
use std::path::{Component, Path};

use crate::write::pad_bodies;
use crate::{
    Body, Candidate, DataItem, DecnInfo, Descriptor, Distinct, Node, Pri, Qualifier, ResMap,
    Schema, Scope, Section,
};

/// What went wrong while building.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BuildError {
    pub what: String,
}

impl fmt::Display for BuildError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.what)
    }
}

impl std::error::Error for BuildError {}

fn err<T>(what: impl Into<String>) -> Result<T, BuildError> {
    Err(BuildError { what: what.into() })
}

fn fit(n: usize, what: &str) -> Result<u16, BuildError> {
    u16::try_from(n).or_else(|_| err(format!("{what} ({n}) doesn't fit 16 bits")))
}

/// The folder a layout's assets live in, when none is given.
pub const DEFAULT_INDEX: &str = "Assets";

/// Qualifier types, by their number in `[mrm_decn_info]`.
const CONTRAST: u16 = 1;
const SCALE: u16 = 2;
const TARGET_SIZE: u16 = 4;
const THEME: u16 = 6;
const ALTERNATE_FORM: u16 = 7;

/// File-name qualifier -> (type, priority), highest priority first. The priorities are the ones
/// Windows' own packages carry (the spike's census).
const QUALIFIERS: [(&str, u16, u16); 6] = [
    ("contrast", CONTRAST, 400),
    ("theme", THEME, 350),
    ("targetsize", TARGET_SIZE, 300),
    ("scale", SCALE, 200),
    ("altform", ALTERNATE_FORM, 100),
    ("alternateform", ALTERNATE_FORM, 100),
];

/// Scale's fallback scores when the default scale is 100, as Windows' own packages carry them
/// (the spike's census): 1000 at the default, less the further away.
const SCALE_FALLBACK: [(u32, u16); 8] = [
    (80, 650),
    (100, 1000),
    (125, 937),
    (140, 900),
    (150, 875),
    (180, 800),
    (200, 750),
    (400, 437),
];

/// The target size that is a package's default, and so scores best.
const DEFAULT_TARGET_SIZE: &str = "256";
/// Fallback score of every value of a qualifier that has no default but one (also the census).
const BEST_SCORE: u16 = 1000;
const OTHER_TARGET_SIZE_SCORE: u16 = 500;

/// The value type of an ASCII path, and where data items sit in the file.
const ASCII_PATH: u8 = 5;
const DATA_ITEM_SECTION: u16 = 4;
const SCHEMA_SECTION: u16 = 2;
const DECN_SECTION: u16 = 0;
const MAP_SECTION: u16 = 3;
/// The seven value types a map lists (data items, `(4, i)`), as Windows' files do.
const MAP_TYPES: u32 = 7;
/// Windows' hierarchical names: items live under this root scope.
const ROOT_SCOPE: &str = "Files";
/// The longest name a node record can hold.
const MAX_NAME_LEN: usize = 255;

/// One qualifier of a file name: its type, priority and UPPERCASE value.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Qual {
    qtype: u16,
    priority: u16,
    value: String,
}

/// The fallback score of a value, or `None` for a scale we have no score for.
fn fallback(qtype: u16, value: &str) -> Option<u16> {
    match qtype {
        TARGET_SIZE => Some(if value == DEFAULT_TARGET_SIZE {
            BEST_SCORE
        } else {
            OTHER_TARGET_SIZE_SCORE
        }),
        SCALE => {
            let scale = value.parse::<u32>().ok()?;
            SCALE_FALLBACK.iter().find(|s| s.0 == scale).map(|s| s.1)
        }
        CONTRAST => Some(if value == "STANDARD" { BEST_SCORE } else { 0 }),
        THEME => Some(if value == "DARK" { BEST_SCORE } else { 0 }),
        _ => Some(0),
    }
}

/// `X.targetsize-24_altform-unplated.png` -> (`X.png`, its qualifiers, highest priority first).
/// A name whose second-to-last dot part isn't a list of `name-value` is no qualifiers at all.
fn split_qualifiers(file: &str) -> Result<(String, Vec<Qual>), BuildError> {
    let parts: Vec<&str> = file.split('.').collect();
    if parts.len() < 3 {
        return Ok((file.to_string(), Vec::new()));
    }
    let mut pairs = Vec::new();
    for token in parts[parts.len() - 2].split('_') {
        let pair = token
            .split_once('-')
            .filter(|(n, v)| {
                !n.is_empty()
                    && n.bytes().all(|b| b.is_ascii_alphabetic())
                    && !v.is_empty()
                    && v.bytes().all(|b| b.is_ascii_alphanumeric())
            })
            .map(|(n, v)| (n.to_ascii_lowercase(), v.to_ascii_uppercase()));
        let Some(pair) = pair else {
            return Ok((file.to_string(), Vec::new()));
        };
        pairs.push(pair);
    }
    let mut quals: Vec<Qual> = Vec::new();
    for (name, value) in pairs {
        let Some(&(_, qtype, priority)) = QUALIFIERS.iter().find(|q| q.0 == name) else {
            return err(format!("{file}: unknown qualifier \"{name}\""));
        };
        if quals.iter().any(|q| q.qtype == qtype) {
            return err(format!("{file}: qualifier \"{name}\" appears twice"));
        }
        if fallback(qtype, &value).is_none() {
            return err(format!(
                "{file}: no fallback score for {name}-{value} yet (default scale 100)"
            ));
        }
        quals.push(Qual {
            qtype,
            priority,
            value,
        });
    }
    quals.sort_by_key(|q| std::cmp::Reverse(q.priority));
    let mut kept = parts[..parts.len() - 2].to_vec();
    kept.push(parts[parts.len() - 1]);
    Ok((kept.join("."), quals))
}

/// makepri's candidate order, descending in this key: by qualifier priority, highest first;
/// within one, values in descending string order and an absent qualifier after every value.
fn order_key(quals: &[Qual]) -> Vec<Option<&str>> {
    let mut priorities: Vec<u16> = QUALIFIERS.iter().map(|q| q.2).collect();
    priorities.sort_unstable_by(|a, b| b.cmp(a));
    priorities.dedup();
    priorities
        .iter()
        .map(|&p| {
            quals
                .iter()
                .find(|q| q.priority == p)
                .map(|q| q.value.as_str())
        })
        .collect()
}

/// A scope of the names tree, children by name.
enum Entry {
    Scope(BTreeMap<String, Entry>),
    Item,
}

/// One candidate of an item: its qualifiers and its value (the file's path, with backslashes).
struct FileCandidate {
    quals: Vec<Qual>,
    value: String,
}

struct Layout {
    nodes: Vec<Node>,
    scopes: Vec<Scope>,
    /// Item index -> node index.
    items: Vec<u16>,
    /// Item index -> the item's path inside `Files` (`Assets/X.png`).
    item_paths: Vec<String>,
}

fn new_node(name: &str, parent: u16, is_scope: bool, index: u16) -> Result<Node, BuildError> {
    if name.chars().count() > MAX_NAME_LEN {
        return err(format!("the name \"{name}\" is longer than {MAX_NAME_LEN}"));
    }
    Ok(Node {
        parent,
        full_len: 0,
        first: 0,
        name_len: name.chars().count() as u8,
        is_scope,
        is_ascii: name.is_ascii(),
        flags_hi: 0,
        offset: 0,
        index,
        name: name.to_string(),
    })
}

impl Layout {
    /// A scope's children sit together, sorted case-insensitively; their own children follow,
    /// each scope visited in turn (docs/pri-plan.md, 2.3).
    fn lay_children(
        &mut self,
        node_i: usize,
        scope_i: usize,
        path: &str,
        tree: &BTreeMap<String, Entry>,
    ) -> Result<(), BuildError> {
        let mut names: Vec<&String> = tree.keys().collect();
        names.sort_by_key(|n| n.to_lowercase());
        let first = self.nodes.len();
        let mut child_scopes = Vec::new();
        for (k, name) in names.iter().enumerate() {
            let full = if path.is_empty() {
                name.to_string()
            } else {
                format!("{path}/{name}")
            };
            let at = fit(self.nodes.len(), "the node count")?;
            let parent = fit(node_i, "a node index")?;
            let node = match &tree[*name] {
                Entry::Scope(sub) => {
                    let index = fit(self.scopes.len(), "the scope count")?;
                    self.scopes.push(Scope {
                        node: at,
                        children: 0,
                        first_child: 0,
                    });
                    child_scopes.push((first + k, self.scopes.len() - 1, full.clone(), sub));
                    new_node(name, parent, true, index)?
                }
                Entry::Item => {
                    let index = fit(self.items.len(), "the item count")?;
                    self.items.push(at);
                    self.item_paths
                        .push(full.strip_prefix("Files/").unwrap_or(&full).to_string());
                    new_node(name, parent, false, index)?
                }
            };
            self.nodes.push(node);
        }
        self.scopes[scope_i].children = fit(names.len(), "a scope's children")?;
        self.scopes[scope_i].first_child = fit(first, "a node index")?;
        for (child_node, child_scope, full, sub) in child_scopes {
            self.lay_children(child_node, child_scope, &full, sub)?;
        }
        Ok(())
    }
}

/// Checks a layout-relative path and splits it into its directories and file name.
fn split_path(file: &str) -> Result<(Vec<&str>, &str), BuildError> {
    let mut parts: Vec<&str> = file.split('/').collect();
    if parts.len() < 2
        || parts
            .iter()
            .any(|p| p.is_empty() || *p == "." || *p == ".." || p.contains(['\\', '\0']))
    {
        return err(format!(
            "\"{file}\": expected a path inside a folder of the layout, with / separators"
        ));
    }
    if !file.is_ascii() {
        return err(format!(
            "\"{file}\": only ASCII file names can be indexed (AsciiPath values)"
        ));
    }
    let leaf = parts.pop().unwrap_or_default();
    Ok((parts, leaf))
}

/// The `Name` of the manifest's `<Identity>` element.
pub fn identity_name(manifest: &str) -> Result<String, BuildError> {
    let text = strip_comments(manifest);
    let mut from = 0;
    while let Some(at) = text[from..].find("<Identity") {
        let start = from + at + "<Identity".len();
        from = start;
        let rest = &text[start..];
        if !rest.starts_with(|c: char| c.is_whitespace() || c == '/' || c == '>') {
            continue; // another element, like <IdentityFoo
        }
        return attribute(rest, "Name")
            .ok_or_else(|| BuildError {
                what: "the manifest's <Identity> has no Name attribute".into(),
            })
            .and_then(|n| {
                if n.is_empty() {
                    err("the manifest's Identity Name is empty")
                } else {
                    Ok(n)
                }
            });
    }
    err("the manifest has no <Identity> element")
}

fn strip_comments(text: &str) -> String {
    let mut out = String::new();
    let mut rest = text;
    while let Some(at) = rest.find("<!--") {
        out.push_str(&rest[..at]);
        match rest[at..].find("-->") {
            Some(end) => rest = &rest[at + end + 3..],
            None => return out,
        }
    }
    out.push_str(rest);
    out
}

/// The value of attribute `wanted` in the attributes `s` opens with, up to the tag's end.
fn attribute(s: &str, wanted: &str) -> Option<String> {
    let mut rest = s;
    loop {
        rest = rest.trim_start();
        if rest.starts_with(['>', '/']) || rest.is_empty() {
            return None;
        }
        let end = rest.find(|c: char| c.is_whitespace() || matches!(c, '=' | '>' | '/'))?;
        let name = &rest[..end];
        rest = rest[end..].trim_start().strip_prefix('=')?.trim_start();
        let quote = rest.chars().next().filter(|c| matches!(c, '"' | '\''))?;
        let close = rest[1..].find(quote)?;
        if name == wanted {
            return Some(rest[1..1 + close].to_string());
        }
        rest = &rest[close + 2..];
    }
}

/// The package's PRI model for `files` (layout-relative paths with `/`, such as
/// `Assets/Square44x44Logo.targetsize-24_altform-unplated.png`) under the identity `name`.
pub fn build(name: &str, files: &[String]) -> Result<Pri, BuildError> {
    if name.is_empty() || name.contains(['\0', '/', '\\']) || name.chars().count() > 0x7FFF {
        return err(format!("\"{name}\" is no usable Identity Name"));
    }
    if files.is_empty() {
        return err("no files to index");
    }

    // Candidates by item, and the names tree.
    let mut by_item: BTreeMap<String, Vec<FileCandidate>> = BTreeMap::new();
    let mut tree: BTreeMap<String, Entry> = BTreeMap::new();
    for file in files {
        let (dirs, leaf) = split_path(file)?;
        let (item_leaf, quals) = split_qualifiers(leaf)?;
        let mut item_path = dirs.join("/");
        item_path.push('/');
        item_path.push_str(&item_leaf);

        let mut node = &mut tree;
        let mut scope_names = vec![ROOT_SCOPE];
        scope_names.extend(dirs.iter().copied());
        for dir in scope_names {
            let entry = node
                .entry(dir.to_string())
                .or_insert_with(|| Entry::Scope(BTreeMap::new()));
            let Entry::Scope(sub) = entry else {
                return err(format!("\"{file}\": \"{dir}\" is both a file and a folder"));
            };
            node = sub;
        }
        if matches!(node.get(&item_leaf), Some(Entry::Scope(_))) {
            return err(format!(
                "\"{file}\": \"{item_leaf}\" is both a file and a folder"
            ));
        }
        node.insert(item_leaf, Entry::Item);

        let cands = by_item.entry(item_path).or_default();
        if cands.iter().any(|c| c.quals == quals) {
            return err(format!(
                "\"{file}\": another file has the same item and qualifiers"
            ));
        }
        cands.push(FileCandidate {
            quals,
            value: file.replace('/', "\\"),
        });
    }

    let mut layout = Layout {
        nodes: vec![new_node("", 0, true, 0)?],
        scopes: vec![Scope {
            node: 0,
            children: 0,
            first_child: 0,
        }],
        items: Vec::new(),
        item_paths: Vec::new(),
    };
    layout.lay_children(0, 0, "", &tree)?;

    // Qualifiers, qualifier sets and decisions. Windows' files all open with an empty Language
    // qualifier and a neutral set; decision 0 is empty and decision 1 the neutral set alone.
    let mut distinct = vec![Distinct {
        env: 0,
        qualifier_type: 0,
        op: 0,
        value_type: 1,
        value_offset: 0,
        value: String::new(),
    }];
    let mut values: Vec<u16> = vec![0];
    let mut quals = vec![Qualifier {
        distinct: 0,
        priority: 0,
        fallback: 0,
        reserved: 0,
    }];
    let mut sets: Vec<Vec<u16>> = vec![Vec::new()];
    let mut decisions: Vec<Vec<u16>> = vec![Vec::new(), vec![0]];

    let mut infos = Vec::new();
    let mut candidates = Vec::new();
    let mut strings: Vec<Vec<u8>> = Vec::new();
    for item_path in &layout.item_paths {
        let mut cands: Vec<&FileCandidate> = by_item.get(item_path).into_iter().flatten().collect();
        cands.sort_by(|a, b| order_key(&b.quals).cmp(&order_key(&a.quals)));
        let mut decision = Vec::new();
        for c in &cands {
            let mut set = Vec::new();
            for q in &c.quals {
                let di = match distinct
                    .iter()
                    .position(|d| d.qualifier_type == q.qtype && d.value == q.value)
                {
                    Some(i) => i,
                    None => {
                        distinct.push(Distinct {
                            env: 2,
                            qualifier_type: q.qtype,
                            op: 0,
                            value_type: 10,
                            value_offset: u32::try_from(values.len())
                                .or_else(|_| err("the qualifier values don't fit 32 bits"))?,
                            value: q.value.clone(),
                        });
                        values.extend(q.value.encode_utf16());
                        values.push(0);
                        distinct.len() - 1
                    }
                };
                let Some(score) = fallback(q.qtype, &q.value) else {
                    return err(format!("no fallback score for {}", q.value));
                };
                let qual = Qualifier {
                    distinct: fit(di, "the distinct qualifier count")?,
                    priority: q.priority,
                    fallback: score,
                    reserved: 0,
                };
                let qi = match quals.iter().position(|x| *x == qual) {
                    Some(i) => i,
                    None => {
                        quals.push(qual);
                        quals.len() - 1
                    }
                };
                set.push(fit(qi, "the qualifier count")?);
            }
            let si = match sets.iter().position(|s| *s == set) {
                Some(i) => i,
                None => {
                    sets.push(set);
                    sets.len() - 1
                }
            };
            decision.push(fit(si, "the qualifier set count")?);
        }
        let di = match decisions.iter().position(|d| *d == decision) {
            Some(i) => i,
            None => {
                decisions.push(decision);
                decisions.len() - 1
            }
        };
        infos.push((
            fit(di, "the decision count")?,
            fit(candidates.len(), "the candidate count")?,
        ));
        for c in cands {
            candidates.push(Candidate::Ref {
                value_type: ASCII_PATH,
                reserved: 0,
                item: fit(strings.len(), "the string count")?,
                section: DATA_ITEM_SECTION,
            });
            let mut s = c.value.clone().into_bytes();
            s.push(0);
            strings.push(s);
        }
    }

    // The index table: the sets' qualifier lists, then the decisions' set lists, reusing a run
    // that is already there.
    let mut index_table: Vec<u16> = Vec::new();
    let mut place = |seq: &[u16]| -> Result<(u16, u16), BuildError> {
        if seq.is_empty() {
            return Ok((0, 0));
        }
        let at = match index_table.windows(seq.len()).position(|w| w == seq) {
            Some(i) => i,
            None => {
                index_table.extend_from_slice(seq);
                index_table.len() - seq.len()
            }
        };
        Ok((fit(at, "the index table")?, fit(seq.len(), "a run")?))
    };
    let set_rows = sets
        .iter()
        .map(|s| place(s))
        .collect::<Result<Vec<_>, _>>()?;
    let decision_rows = decisions
        .iter()
        .map(|d| place(d))
        .collect::<Result<Vec<_>, _>>()?;
    if index_table.is_empty() {
        index_table.push(0);
    }

    let n_items = fit(layout.items.len(), "the item count")?;
    let mut data = Vec::new();
    let mut strs = Vec::new();
    for s in &strings {
        strs.push((
            fit(data.len(), "a string's offset")?,
            fit(s.len(), "a string's length")?,
        ));
        data.extend_from_slice(s);
    }

    let section = |id: &[u8; 16], body| Section {
        id: *id,
        toc_flags: 0,
        toc_section_flags: 0,
        toc_qualifier: 0,
        header_qualifier: 0,
        header_flags: 0,
        header_section_flags: 0,
        header_u28: 0,
        body,
    };
    let mut pri = Pri {
        magic: "mrm_pri2".into(),
        header_u8: 0,
        header_u10: 1,
        header_u26: 0xFFFF,
        header_u28: 0,
        sections: vec![
            section(
                b"[mrm_decn_info]\0",
                Body::Decisions(DecnInfo {
                    decisions: decision_rows,
                    sets: set_rows,
                    quals,
                    distinct,
                    index_table,
                    values: values.iter().flat_map(|u| u.to_le_bytes()).collect(),
                    pad: Vec::new(),
                }),
            ),
            section(
                b"[mrm_pridescex]\0",
                Body::Descriptor(Descriptor {
                    flags: 1,
                    included_file_list: 0xFFFF,
                    u4: 0,
                    primary_map: MAP_SECTION,
                    u18: 0,
                    schemas: vec![SCHEMA_SECTION],
                    decn_infos: vec![DECN_SECTION],
                    res_maps: vec![MAP_SECTION],
                    file_lists: Vec::new(),
                    data_items: vec![DATA_ITEM_SECTION],
                    tail: vec![0; 4],
                }),
            ),
            section(
                b"[mrm_hschemaex] ",
                Body::Schema(Some(Schema {
                    extended: true,
                    hnames_id: Some(*b"[def_hnamesx]  \0"),
                    major: 1,
                    minor: 0,
                    checksum: 0,
                    unique_name: format!("ms-appx://{name}/"),
                    name: name.to_string(),
                    max_full_path: 0,
                    total: 0,
                    nodes: layout.nodes,
                    scopes: layout.scopes,
                    items: layout.items,
                    names_uni: Vec::new(),
                    names_ascii: Vec::new(),
                    pad: Vec::new(),
                })),
            ),
            section(
                b"[mrm_res_map2_]\0",
                Body::Map(ResMap {
                    v2: true,
                    env: Vec::new(),
                    env_count: 0,
                    schema_section: SCHEMA_SECTION,
                    schema_ref: Vec::new(),
                    decn_section: DECN_SECTION,
                    types: (0..MAP_TYPES).map(|i| (4, i)).collect(),
                    item_to_group: vec![(0, 0)],
                    groups: vec![(n_items, 0)],
                    infos,
                    large: Vec::new(),
                    candidates,
                    data: Vec::new(),
                    pad: Vec::new(),
                }),
            ),
            section(
                b"[mrm_dataitem] \0",
                Body::Data(DataItem {
                    u0: 0,
                    strs,
                    blobs: Vec::new(),
                    data,
                    pad: Vec::new(),
                }),
            ),
        ],
    };
    pad_bodies(&mut pri).map_err(|e| BuildError { what: e.what })?;
    Ok(pri)
}

/// The files under the `index` folders of `layout`, as layout-relative paths with `/`, sorted.
/// `exclude` (the output file) is never listed.
pub fn scan(
    layout: &Path,
    index: &[String],
    exclude: Option<&Path>,
) -> Result<Vec<String>, BuildError> {
    let exclude = exclude.and_then(|p| p.canonicalize().ok());
    let mut files = Vec::new();
    for dir in index {
        let rel = Path::new(dir);
        if rel.components().any(|c| !matches!(c, Component::Normal(_))) {
            return err(format!("\"{dir}\": expected a folder inside the layout"));
        }
        let name = rel
            .components()
            .map(|c| c.as_os_str().to_str().map(String::from))
            .collect::<Option<Vec<_>>>()
            .map(|p| p.join("/"));
        let Some(name) = name else {
            return err(format!("\"{dir}\" is not valid UTF-8"));
        };
        walk(&layout.join(rel), &name, exclude.as_deref(), &mut files)?;
    }
    files.sort();
    files.dedup();
    Ok(files)
}

fn walk(
    dir: &Path,
    rel: &str,
    exclude: Option<&Path>,
    files: &mut Vec<String>,
) -> Result<(), BuildError> {
    let read = std::fs::read_dir(dir).map_err(|e| BuildError {
        what: format!("{}: {e}", dir.display()),
    })?;
    for entry in read {
        let entry = entry.map_err(|e| BuildError {
            what: format!("{}: {e}", dir.display()),
        })?;
        let Some(name) = entry.file_name().to_str().map(String::from) else {
            return err(format!("{}: a name that is not UTF-8", dir.display()));
        };
        let path = entry.path();
        let kind = entry.file_type().map_err(|e| BuildError {
            what: format!("{}: {e}", path.display()),
        })?;
        let child = format!("{rel}/{name}");
        if kind.is_dir() {
            walk(&path, &child, exclude, files)?;
        } else if kind.is_file() || (kind.is_symlink() && path.is_file()) {
            if exclude.is_some() && path.canonicalize().ok().as_deref() == exclude {
                continue;
            }
            files.push(child);
        }
    }
    Ok(())
}

/// The PRI of a layout directory: `name` (or the manifest's Identity Name) over the files in
/// its `index` folders (`Assets` when empty), leaving out `exclude`.
pub fn build_layout(
    layout: &Path,
    name: Option<&str>,
    index: &[String],
    exclude: Option<&Path>,
) -> Result<Pri, BuildError> {
    let name = match name {
        Some(n) => n.to_string(),
        None => {
            let path = layout.join("AppxManifest.xml");
            let text = std::fs::read_to_string(&path).map_err(|e| BuildError {
                what: format!("{}: {e} (give --name instead)", path.display()),
            })?;
            identity_name(&text)?
        }
    };
    let default = [DEFAULT_INDEX.to_string()];
    let index = if index.is_empty() { &default } else { index };
    build(&name, &scan(layout, index, exclude)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn q(file: &str) -> Vec<(u16, u16, String)> {
        split_qualifiers(file)
            .unwrap()
            .1
            .into_iter()
            .map(|q| (q.qtype, q.priority, q.value))
            .collect()
    }

    #[test]
    fn qualifiers_from_names() {
        assert_eq!(
            split_qualifiers("X.scale-200.png").unwrap(),
            (
                "X.png".to_string(),
                vec![Qual {
                    qtype: SCALE,
                    priority: 200,
                    value: "200".into()
                }]
            )
        );
        assert_eq!(
            q("X.targetsize-24_altform-unplated.png"),
            vec![
                (TARGET_SIZE, 300, "24".into()),
                (ALTERNATE_FORM, 100, "UNPLATED".into())
            ]
        );
        // Sorted by priority whatever the name's order.
        assert_eq!(
            q("X.altform-unplated_targetsize-24.png")[0],
            (TARGET_SIZE, 300, "24".into())
        );
        assert_eq!(
            q("X.contrast-high.png"),
            vec![(CONTRAST, 400, "HIGH".into())]
        );
        assert_eq!(q("X.theme-dark.png"), vec![(THEME, 350, "DARK".into())]);
        assert_eq!(q("X.png"), vec![]);
        // Not a qualifier list: the whole name stays.
        assert_eq!(
            split_qualifiers("a.b.png").unwrap(),
            ("a.b.png".to_string(), vec![])
        );
    }

    #[test]
    fn qualifier_errors() {
        assert!(split_qualifiers("X.flavour-sweet.png").is_err());
        assert!(split_qualifiers("X.scale-123.png").is_err());
        assert!(split_qualifiers("X.scale-100_scale-200.png").is_err());
    }

    #[test]
    fn candidate_order() {
        let files: Vec<String> = [
            "X.png",
            "X.scale-200.png",
            "X.targetsize-24.png",
            "X.targetsize-256.png",
            "X.targetsize-96.png",
            "X.targetsize-24_altform-unplated.png",
            "X.targetsize-24_altform-lightunplated.png",
            "X.contrast-high.png",
        ]
        .iter()
        .map(|f| format!("Assets/{f}"))
        .collect();
        let pri = build("Test.Pkg", &files).unwrap();
        let listing = pri.listing().unwrap();
        let order: Vec<&str> = listing
            .lines()
            .filter_map(|l| l.split('\\').nth(1))
            .collect();
        assert_eq!(
            order,
            [
                "X.contrast-high.png",
                "X.targetsize-96.png",
                "X.targetsize-256.png",
                "X.targetsize-24_altform-unplated.png",
                "X.targetsize-24_altform-lightunplated.png",
                "X.targetsize-24.png",
                "X.scale-200.png",
                "X.png",
            ]
        );
    }

    #[test]
    fn duplicates_and_bad_paths() {
        let f = |s: &[&str]| -> Vec<String> { s.iter().map(|s| s.to_string()).collect() };
        assert!(build("P", &f(&["Assets/X.png", "Assets/X.scale-100.png"])).is_ok());
        assert!(build(
            "P",
            &f(&["Assets/X.scale-100.png", "Assets/X.SCALE-100.png"])
        )
        .is_err());
        assert!(build(
            "P",
            &f(&[
                "Assets/X.targetsize-24_altform-unplated.png",
                "Assets/X.altform-unplated_targetsize-24.png"
            ])
        )
        .is_err());
        assert!(build("P", &f(&["X.png"])).is_err());
        assert!(build("P", &f(&["Assets/../X.png"])).is_err());
        assert!(build("", &f(&["Assets/X.png"])).is_err());
        assert!(build("P", &[]).is_err());
    }

    #[test]
    fn identity_from_manifest() {
        let m = "<?xml version=\"1.0\"?>\n<!-- <Identity Name=\"Wrong\"/> -->\n<Package>\n  <IdentityX Name=\"No\"/>\n  <Identity\n    Publisher='CN=x'\n    Name = 'My.App'  Version=\"1.0.0.0\" />\n</Package>";
        assert_eq!(identity_name(m).unwrap(), "My.App");
        assert_eq!(
            identity_name("<Package><Identity Name=\"A.B\"></Identity></Package>").unwrap(),
            "A.B"
        );
        assert!(identity_name("<Package><Identity Version=\"1\"/></Package>").is_err());
        assert!(identity_name("<Package/>").is_err());
    }
}
