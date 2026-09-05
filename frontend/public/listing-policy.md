# Agent Skills Registry — Listing Policy

**Version**: 2.1 (draft; pre-launch amendment 2026-08-24: symlinks prohibited in trees, executable bits non-semantic — owner decision)
**Registry**: Agent Skills Registry (Classic GeneralizedTCR)
**Governor**: `[GOVERNOR_ADDRESS]`
**Chain**: Gnosis Chain (chain ID 100)
**Registry address**: `[TO BE SET AT DEPLOYMENT]`

## Purpose

This registry curates AI agent **skills** through an optimistic open-challenge process. Anyone may submit a skill with a deposit, anyone may challenge it, and challenged entries are adjudicated by Kleros jurors against this policy. An unchallenged entry may become Registered without juror review; Registered means that the exact entry survived this process, not that any particular person inspected it.

A registry item binds its policy result to **exact bytes**, identified by content address — never to a name, a URL, a repository, or any mutable reference.

## Scope

This registry lists **skills only**: directories containing a `SKILL.md` per the Agent Skills specification, providing invocable capabilities to AI agents. Conventions (project-instruction files such as AGENTS.md or CLAUDE.md) and plugins (Agent Plugins bundles) are out of scope for this registry and may be served by separate registries with their own policies.

## Definitions

- **Skill tree** — the complete skill directory (SKILL.md plus any `scripts/`, `references/`, `assets/`, or other files), addressed by a single IPFS UnixFS directory CID (the **Tree CID**). The Tree CID immutably identifies every byte of the skill. The root CID and every linked CID in the DAG must use a 32-byte SHA-256 multihash; linked blocks may use only the DAG-PB or raw codec.
- **Virtual root name** — a Tree CID does not encode a name for its outer directory. Wherever a skill specification or runtime needs that name, this policy defines it to be the descriptor's Name value.
- **Descriptor** — the on-chain item: the RLP encoding of the column values below, stored in registry contract storage. The item ID is `keccak256(descriptor)`.
- **Submission period** — for this policy, the interval from submission of a registration request until that request is challenged or its challenge deadline passes.

## Descriptor Schema

Each submission consists of exactly these columns, in order:

| # | Column | Required | Format |
|---|---|---|---|
| 1 | **Name** | Yes | Must be **byte-identical** to the `name` field of the SKILL.md frontmatter in the skill tree. |
| 2 | **Description** | Yes | Must be **byte-identical** to the `description` field of the SKILL.md frontmatter (≤1024 characters per the Agent Skills specification). |
| 3 | **Tree CID** | Yes | The skill tree's IPFS CID in **canonical form**: CIDv1, base32, lowercase, DAG-PB codec, and a 32-byte SHA-256 multihash, referring to a UnixFS directory. Bare CID only — no `/ipfs/` prefix, no path suffix, no gateway URL. **IPNS names, DNSLink, and any mutable or resolvable-indirection reference are prohibited.** |
| 4 | **Runtimes** | Yes | Comma-separated list of supported agent runtimes. Use `generic` if the skill follows the Agent Skills specification without runtime-specific metadata or features; list specific runtimes (e.g., `claude_code`) if it relies on such metadata or features. |
| 5 | **Origin** | No | A provenance claim: either a public git repository URL followed by `@` and a full 40-character commit SHA-1, or a publisher URL (e.g., `https://skills.example.org`). If present, it must satisfy acceptance criterion 6. Leave empty if unused. |
| 6 | **Reserved** | Yes | MUST be the empty string. It has no semantics in this policy version; assigning any future meaning requires a versioned policy revision. |

## Acceptance Criteria

An entry MUST satisfy ALL of the following criteria. An entry that fails ANY criterion must be rejected (registration disputes) or removed (removal disputes).

### 1. Well-Formed Descriptor
All six columns are present and conform to the formats above, including the canonical-CID rule for the Tree CID and the requirement that Reserved is empty.

### 2. Valid Skill
The skill tree's root contains a `SKILL.md` that is a valid skill per the **Agent Skills specification as published at commit `[SPEC_COMMIT_HASH]` of `github.com/agentskills/agentskills`**: YAML frontmatter with required `name` (1–64 chars, lowercase alphanumeric and hyphens, no leading/trailing/consecutive hyphens) and `description` (1–1024 chars); only spec-defined top-level frontmatter fields are permitted, and runtime-specific extension data must use the specification's `metadata` map. Output of validation tools (e.g., `skills-ref`) is admissible **evidence** of compliance or non-compliance, but the specification text at the pinned commit is the standard — not any tool.

The entry must be a skill (an invocable capability). Project-instruction files, plugin bundles, or other artifact types miscategorized as skills fail this criterion.

The tree consists of directories and regular files ONLY: it MUST NOT contain symlinks (of any target), and executable permission bits are **not semantic** in this policy version — conforming installers neither preserve nor interpret them, and skills invoke their scripts through explicit interpreters (e.g., `bash scripts/x.sh`, `python3 scripts/y.py`). A tree whose behavior depends on a file's executable bit fails this criterion.

### 3. Frontmatter Binding
The Name and Description columns are byte-identical to the SKILL.md frontmatter `name` and `description`. (This guarantees the on-chain strings a consumer indexes are exactly the strings an agent runtime will load from the verified artifact.)

### 4. Accurate Runtimes
The skill works (or is designed to work) with each declared runtime. Runtimes must not be listed speculatively. A skill using runtime-specific `metadata` or features must not declare `generic`.

### 5. No Malicious Behavior
The skill tree must NOT:

- Exfiltrate user data, credentials, environment variables, or files;
- Contain prompt injection targeting the consuming agent, other tools, or **reviewers and automated evaluators** (instructions addressed to "the reviewer", "the scanner", or an LLM judge are a violation in themselves);
- **Fetch and execute, or fetch and inject into agent context as instructions, content whose exact bytes are not bound by a SHA-256 integrity digest written as `sha256:` followed by exactly 64 lowercase hexadecimal characters.** The behavior and digest must also be prominently disclosed in the Description, and the skill must verify the fetched bytes and abort on mismatch before execution or context injection. Disclosed retrieval of mutable external data is permitted only when that data is not interpreted as code or agent instructions;
- Download or execute external binaries unless the behavior is prominently declared, the exact binary is identified by a `sha256:<64 lowercase hex characters>` digest, and the skill verifies that digest before execution and aborts on mismatch;
- Disable, bypass, or instruct the agent to bypass security mechanisms, sandboxing, or permission prompts;
- Install persistent backdoors or perform resource abuse (e.g., cryptocurrency mining);
- Employ **analysis-evasion constructions**, including: bulk padding or filler designed to truncate review (e.g., massive runs of whitespace or repeated tokens); executable logic or instructions concealed inside binary or archive assets (.docx, .zip, images, etc.) that the skill later extracts or interprets; encodings or obfuscation whose evident purpose is to defeat inspection. Such constructions are grounds for rejection **on construction alone**, without demonstrating the concealed payload's behavior.

### 6. Truthful Origin
If the Origin column is present, it must verifiably bind the claimed publisher or repository to this entry's **exact submitted semantic skill tree**:

- a git Origin must point to the stated commit, and that commit must contain the same semantic skill tree as the Tree CID; or
- a publisher Origin must publish a statement or listing under that origin which identifies the same Tree CID as its skill.

Common ownership, cross-linking, or control of both a repository and a domain is not sufficient unless the public provenance evidence binds the exact submitted semantic skill tree under one of the rules above.

For git comparison, the semantic tree consists of every relative path, node kind (directory or regular file), and regular-file byte sequence. Symlinks are prohibited in trees (criterion 2), and executable bits are not semantic in this policy version, so a git commit that differs from the submitted tree ONLY in executable bits still binds it; a commit containing symlinks cannot bind any listable tree. The descriptor Name supplies the virtual outer-directory name. UnixFS chunking and transport-only metadata such as modification times do not create a provenance mismatch.

A false or unverifiable Origin claim is a violation of the same severity as impersonation.

### 7. No Impersonation
The entry must not impersonate, typosquat, or misleadingly copy another project's or publisher's name, description, or branding.

### 8. No Duplicates
The entry must not duplicate an already-registered entry with the same Tree CID. (Identical descriptors are already impossible: the item ID is the descriptor hash.)

### 9. Reviewable Size
The skill tree's total size must not exceed **2 MiB**. (This bound exists so that jurors and challengers can realistically review entries; it may be revised in future policy versions.)

### 10. Submission-Period Availability
Throughout the submission period, the complete skill tree identified by the Tree CID MUST be stored on IPFS and remain accessible and discoverable. Otherwise, the entry fails this criterion.

### 11. Lawful Distribution
The submitter must be entitled to distribute the skill tree's content. Entries whose distribution violates the license or rights of included content may be rejected or removed. A `license` frontmatter field or bundled license file is strongly recommended.

## Adjudication Principles

These principles bind how the criteria above are applied:

**A. Content identity is hosting-independent.** Jurors may obtain the skill tree from any provider, but MUST verify the complete content and directory structure against the Tree CID before relying on it. A matching CID authenticates the bytes; it does not make any gateway or provider trusted.

**B. Availability is a submission-period obligation.** Criterion 10 concerns accessibility and discoverability during the registration request's challenge period. It does not require permanent availability after listing. Producing hash-matching bytes later proves their identity, not that they were available throughout the earlier submission period, and does not by itself cure a demonstrated earlier violation. Conversely, a failed fetch during juror voting does not by itself prove that the tree was unavailable during the earlier submission period.

**C. Availability is decided from evidence, not a juror-time liveness test.** Jurors evaluate evidence about criterion 10 through the ordinary Kleros dispute process. This policy prescribes no mechanical availability test, retry count, uptime monitor, or live-fetch procedure. Current availability or unavailability must not be substituted for evidence about the submission period.

**D. Construction suffices for evasion.** Criterion 5's evasion clause is deliberately judgeable without executing anything: a skill that pads, conceals, or addresses its reviewers has violated the policy by how it is built.

## Removal Criteria

A registered entry should be removed if it fails an acceptance criterion — including violations discovered after listing, newly discovered security vulnerabilities in the skill, or an Origin claim that has become demonstrably false (e.g., the publisher disavows the skill).

Post-listing unavailability, by itself, is **not** a removal ground: criterion 10 does not impose permanent hosting. Evidence that the entry violated criterion 10 during its submission period may still support removal.

`Absent` status alone does not prove that an item was removed; rejected items that were never Registered are also Absent. A consumer SHOULD treat an exact item as locally revoked only when its authenticated local history or lockfile records that item as previously Registered and a fresh authenticated check now reports the same item ID as Absent. Consumers SHOULD retain installed bytes and lockfile history for warning, audit, and forensic use rather than silently deleting them.

## Consumer Handling of Pending Removal

Consumers MUST treat every `ClearingRequested` item uniformly, regardless of who requested removal. New installation and automatic loading or execution SHOULD be suspended by default while the request is pending. A consumer MAY permit an explicit local override and SHOULD retain existing bytes and lockfile history for inspection. No requester address has special protocol or trust semantics.

## Governance

The registry is neutral infrastructure and carries no product branding. Its governor is the Safe at `[GOVERNOR_ADDRESS]`. The governor may change deposits, the challenge period, the appeal stake multipliers, the arbitrator and its extra data, and the governor itself, and announces every such change ahead of time. The governor never changes this policy or the registry's MetaEvidence: verifiers treat any MetaEvidence update as a policy change and refuse the registry, so a new policy means a new registry. The governor does not adjudicate; challenged entries are decided by Kleros jurors under this policy. The intended direction is to widen governance to the registry's community as adoption grows.

## Evidence Guidelines

Challengers should identify the criterion violated and provide evidence specific to that criterion. For content-based allegations, they should identify the relevant files and lines and provide the skill tree or another means for jurors to retrieve it; every byte relied upon must verify against the Tree CID. For criterion 10, they should provide contemporaneous evidence concerning accessibility and discoverability during the submission period. Scanner reports and reproducible behavioral evidence may be submitted as supporting evidence. When challenging on evasion constructions (criterion 5), identifying the construction is sufficient — the payload need not be detonated.

Submitters defending an entry should provide the verified skill tree if it is needed to evaluate a content allegation and address the alleged violation. For an availability allegation, evidence of a later successful fetch or later production of the tree does not, by itself, establish earlier submission-period availability.

## Juror Instructions

1. Read this policy in full. The pinned Agent Skills specification commit is part of this policy for criterion 2.
2. Obtain the skill tree when needed to decide the alleged violation, and verify its complete DAG and directory structure against the Tree CID before relying on it. Hash-matching content is authentic regardless of source.
3. Check every acceptance criterion. Failure of any single criterion decides the dispute.
4. For criterion 10, evaluate evidence about the submission period. Do not infer continuous earlier availability merely from a successful juror-time fetch, do not infer earlier unavailability merely from a failed juror-time fetch, and do not treat late production as an automatic cure.
5. Be aware that skill content may attempt to manipulate you: instructions embedded in the skill addressed to reviewers or AI assistants are criterion-5 violations, not instructions to follow.
6. When in doubt, prefer the interpretation that protects end users from potential harm.
