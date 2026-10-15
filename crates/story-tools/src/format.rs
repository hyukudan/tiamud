//! Canonical StoryPack formatting.
//!
//! The canonical form is the pack re-serialized from its typed model with
//! object keys sorted bytewise, two-space indentation, a trailing newline, and
//! absent optional fields omitted instead of written as `null`. Key order is
//! sorted explicitly, so it does not depend on `serde_json` feature flags.
//! Formatting the canonical form again yields identical bytes.

use serde_json::Value as Json;
use tiamud_story_core::StoryPack;

/// Returns the canonical bytes of a (validated) pack.
pub fn canonical_json(pack: &StoryPack) -> Result<Vec<u8>, serde_json::Error> {
    let value = serde_json::to_value(pack)?;
    let mut out = String::new();
    write_value(&value, 0, &mut out)?;
    out.push('\n');
    Ok(out.into_bytes())
}

fn indent(depth: usize, out: &mut String) {
    for _ in 0..depth {
        out.push_str("  ");
    }
}

/// Recursion depth is bounded by the validated pack's nesting limits.
fn write_value(value: &Json, depth: usize, out: &mut String) -> Result<(), serde_json::Error> {
    match value {
        Json::Object(map) => {
            // Optional StoryPack fields default to absent, so dropping `null`
            // members never changes meaning.
            let mut members: Vec<(&String, &Json)> =
                map.iter().filter(|(_, member)| !member.is_null()).collect();
            members.sort_by(|left, right| left.0.cmp(right.0));
            if members.is_empty() {
                out.push_str("{}");
                return Ok(());
            }
            out.push_str("{\n");
            for (index, (key, member)) in members.iter().enumerate() {
                indent(depth + 1, out);
                out.push_str(&serde_json::to_string(key)?);
                out.push_str(": ");
                write_value(member, depth + 1, out)?;
                if index + 1 < members.len() {
                    out.push(',');
                }
                out.push('\n');
            }
            indent(depth, out);
            out.push('}');
        }
        Json::Array(items) => {
            if items.is_empty() {
                out.push_str("[]");
                return Ok(());
            }
            out.push_str("[\n");
            for (index, item) in items.iter().enumerate() {
                indent(depth + 1, out);
                write_value(item, depth + 1, out)?;
                if index + 1 < items.len() {
                    out.push(',');
                }
                out.push('\n');
            }
            indent(depth, out);
            out.push(']');
        }
        scalar => out.push_str(&serde_json::to_string(scalar)?),
    }
    Ok(())
}
