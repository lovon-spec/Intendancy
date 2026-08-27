//! Deterministic local policy checks at install time (review finding 10, the
//! non-owner-gated subset): the verified tree MUST carry a root-level
//! `SKILL.md`, and the descriptor's Name and Description columns MUST
//! byte-match the corresponding frontmatter values — the listing policy's
//! byte-equality rule. Subjective policy adjudication (Origin semantics,
//! malware, quality) deliberately stays the TCR's signal, not a local check.

use std::collections::BTreeMap;

use eyre::{bail, Result};

use crate::car::{planned_file_bytes, Cid, InstallPlan};
use crate::schema::Descriptor;

/// The frontmatter fields the binding compares. Parsed with a REAL YAML parser
/// (official Agent Skills client guidance: parse the YAML block — a raw
/// key-line scanner disagrees with clients on quotes, comments, block scalars,
/// CRLF, and duplicates). serde's derive rejects duplicate `name`/`description`
/// keys; non-string values fail typed; unknown frontmatter fields are the
/// spec's business, not the binding's, and are ignored here.
#[derive(serde::Deserialize)]
struct Frontmatter {
    name: String,
    description: String,
}

/// Extract the frontmatter block: the document must START with a `---` line and
/// the block ends at the next `---` line (CRLF tolerated on both).
fn frontmatter_block(content: &str) -> Result<&str> {
    let mut lines = content.split_inclusive('\n');
    let first = lines
        .next()
        .ok_or_else(|| eyre::eyre!("SKILL.md is empty"))?;
    if first.trim_end_matches(['\r', '\n']) != "---" {
        bail!("SKILL.md does not start with a `---` frontmatter block");
    }
    let start = first.len();
    let mut offset = start;
    for line in lines {
        if line.trim_end_matches(['\r', '\n']) == "---" {
            return Ok(&content[start..offset]);
        }
        offset += line.len();
    }
    bail!("SKILL.md frontmatter block is unterminated")
}

fn parse_frontmatter(content: &str) -> Result<Frontmatter> {
    let block = frontmatter_block(content)?;
    serde_yaml::from_str(block).map_err(|e| eyre::eyre!("SKILL.md frontmatter YAML: {e}"))
}

/// Enforce the descriptor↔SKILL.md binding over the PREFLIGHTED plan, before
/// anything is written.
pub fn verify_skill_binding(
    plan: &InstallPlan,
    blocks: &BTreeMap<Cid, Vec<u8>>,
    descriptor: &Descriptor,
) -> Result<()> {
    let skill = plan
        .files
        .iter()
        .find(|f| f.rel.as_os_str() == "SKILL.md")
        .ok_or_else(|| eyre::eyre!("tree has no root-level SKILL.md (required by policy)"))?;
    let bytes = planned_file_bytes(skill, blocks)?;
    let content =
        std::str::from_utf8(&bytes).map_err(|_| eyre::eyre!("SKILL.md is not valid UTF-8"))?;
    let fm = parse_frontmatter(content)?;
    if fm.name != descriptor.name {
        bail!(
            "SKILL.md frontmatter name {:?} does not match the descriptor Name {:?}",
            fm.name,
            descriptor.name
        );
    }
    if fm.description != descriptor.description {
        bail!(
            "SKILL.md frontmatter description does not match the descriptor \
             Description ({:?} vs {:?})",
            fm.description,
            descriptor.description
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::parse_frontmatter;

    #[test]
    fn yaml_frontmatter_semantics() {
        // Plain scalars.
        let fm = parse_frontmatter("---\nname: my-skill\ndescription: Does things.\n---\nBody.\n")
            .unwrap();
        assert_eq!(fm.name, "my-skill");
        assert_eq!(fm.description, "Does things.");
        // Quoted values and comments parse SEMANTICALLY (quotes stripped).
        let fm = parse_frontmatter(
            "---\nname: \"my-skill\" # comment\ndescription: 'Does things.'\n---\n",
        )
        .unwrap();
        assert_eq!(fm.name, "my-skill");
        assert_eq!(fm.description, "Does things.");
        // CRLF documents.
        let fm = parse_frontmatter("---\r\nname: a\r\ndescription: b\r\n---\r\n").unwrap();
        assert_eq!(fm.name, "a");
        // Block scalar descriptions are strings too.
        let fm =
            parse_frontmatter("---\nname: a\ndescription: |-\n  Multi\n  line\n---\n").unwrap();
        assert_eq!(fm.description, "Multi\nline");
        // Duplicate keys reject.
        assert!(parse_frontmatter("---\nname: a\nname: b\ndescription: c\n---\n").is_err());
        // Non-string types reject.
        assert!(parse_frontmatter("---\nname: [1,2]\ndescription: c\n---\n").is_err());
        // Structure errors.
        assert!(parse_frontmatter("no frontmatter").is_err());
        assert!(parse_frontmatter("---\nname: x\n").is_err(), "unterminated");
        assert!(parse_frontmatter("---\ndescription: only\n---\n").is_err());
    }
}
