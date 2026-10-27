//! Each item's candidates as qualifier strings, in file order.

use std::collections::HashMap;
use std::fmt::Write;

use crate::{Body, Candidate, DecnInfo, Error, Pri, ResMap, Schema};

fn bad(what: String) -> Error {
    Error {
        offset: 0,
        expected: what,
    }
}

fn schema_at(pri: &Pri, i: u16) -> Result<&Schema, Error> {
    match pri.sections.get(i as usize).map(|s| &s.body) {
        Some(Body::Schema(Some(s))) => Ok(s),
        _ => Err(bad(format!("section {i} to be a schema"))),
    }
}

fn decn_at(pri: &Pri, i: u16) -> Result<&DecnInfo, Error> {
    match pri.sections.get(i as usize).map(|s| &s.body) {
        Some(Body::Decisions(d)) => Ok(d),
        _ => Err(bad(format!("section {i} to be decision info"))),
    }
}

/// A qualifier set as `Type=VALUE(priority/fallback)+...`, `neutral` when empty.
fn set_text(dn: &DecnInfo, set: usize) -> Result<String, Error> {
    let &(first, n) = dn
        .sets
        .get(set)
        .ok_or_else(|| bad(format!("qualifier set {set}")))?;
    let range = first as usize..first as usize + n as usize;
    let indices = dn
        .index_table
        .get(range)
        .ok_or_else(|| bad(format!("index table run for set {set}")))?;
    let mut parts = Vec::new();
    for &qi in indices {
        let q = dn
            .quals
            .get(qi as usize)
            .ok_or_else(|| bad(format!("qualifier {qi}")))?;
        let d = dn
            .distinct
            .get(q.distinct as usize)
            .ok_or_else(|| bad(format!("distinct qualifier {}", q.distinct)))?;
        parts.push(format!(
            "{}={}({}/{})",
            d.type_name(),
            d.value,
            q.priority,
            q.fallback
        ));
    }
    Ok(if parts.is_empty() {
        "neutral".to_string()
    } else {
        parts.join("+")
    })
}

/// A candidate's value: a data item's string as ASCII, one replacement character per other byte.
fn value_text(pri: &Pri, c: &Candidate) -> Result<String, Error> {
    match c {
        Candidate::Ref { item, section, .. } => {
            let Some(Body::Data(d)) = pri.sections.get(*section as usize).map(|s| &s.body) else {
                return Err(bad(format!("section {section} to be a data item")));
            };
            let values = d.values();
            let v = values
                .get(*item as usize)
                .ok_or_else(|| bad(format!("value {item} in section {section}")))?;
            let end = v.iter().rposition(|&b| b != 0).map_or(0, |e| e + 1);
            Ok(v[..end]
                .iter()
                .map(|&b| if b < 0x80 { b as char } else { '\u{FFFD}' })
                .collect())
        }
        Candidate::Embedded { value, .. } => Ok(format!("embedded {value}")),
    }
}

fn map_listing(pri: &Pri, rm: &ResMap, out: &mut String) -> Result<(), Error> {
    let hs = schema_at(pri, rm.schema_section)?;
    let dn = decn_at(pri, rm.decn_section)?;
    let item_node: HashMap<u16, usize> = hs
        .nodes
        .iter()
        .enumerate()
        .filter(|(_, n)| !n.is_scope)
        .map(|(i, n)| (n.index, i))
        .collect();
    for &(first_item, gi) in &rm.item_to_group {
        let (n, first_info) = match rm.groups.get(gi as usize) {
            Some(&(n, f)) => (n as usize, f as usize),
            None => (1, gi as usize - rm.groups.len()),
        };
        for k in 0..n {
            let item = first_item as usize + k;
            let &(dec, first_cand) = rm
                .infos
                .get(first_info + k)
                .ok_or_else(|| bad(format!("info {}", first_info + k)))?;
            let node = u16::try_from(item)
                .ok()
                .and_then(|i| item_node.get(&i))
                .ok_or_else(|| bad(format!("a node for item {item}")))?;
            let &(f, nsets) = dn
                .decisions
                .get(dec as usize)
                .ok_or_else(|| bad(format!("decision {dec}")))?;
            let _ = writeln!(out, "{} decision {dec}", hs.full_name(*node));
            let sets = dn
                .index_table
                .get(f as usize..f as usize + nsets as usize)
                .ok_or_else(|| bad(format!("index table run for decision {dec}")))?;
            for (j, &si) in sets.iter().enumerate() {
                let c = rm
                    .candidates
                    .get(first_cand as usize + j)
                    .ok_or_else(|| bad(format!("candidate {}", first_cand as usize + j)))?;
                let _ = writeln!(
                    out,
                    "    {} -> {}",
                    set_text(dn, si as usize)?,
                    value_text(pri, c)?
                );
            }
        }
    }
    Ok(())
}

pub fn listing(pri: &Pri) -> Result<String, Error> {
    let mut out = String::new();
    for s in &pri.sections {
        if let Body::Map(rm) = &s.body {
            map_listing(pri, rm, &mut out)?;
        }
    }
    Ok(out)
}
