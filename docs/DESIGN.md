# Intendhub Design Document

## What Is Intendhub?

Intendhub is a decentralized, community-curated registry for AI agent **skills**, **MCP servers**, and **plugins**. It uses a Kleros Curate Token Curated Registry (TCR) on Gnosis Chain to provide a trust layer that sits between existing distribution channels (Smithery, ClawHub, skills.sh, Claude marketplace, npm) and end users.

Anyone can submit entries. Anyone can challenge entries and claim the submitter's deposit if the entry is found to be malicious, fraudulent, or non-compliant. Disputes are resolved by randomly selected Kleros jurors.

## The Problem

As of March 2026, the AI agent tool ecosystem has standardized on formats (SKILL.md for skills, MCP for tools, AGENTS.md for project instructions) but trust remains broken:

- **Permissionless registries get compromised.** ClawHub (OpenClaw's registry): Snyk found 36.8% of 3,984 skills had security flaws, 13.4% had critical issues. 1,467 malicious skills, 91% combining prompt injection with traditional malware.
- **Centralized review doesn't scale.** Docker MCP Catalog has the strongest trust model (signed images, SBOMs, PR-based review) but only covers ~300 servers out of 10,000+.
- **GitHub auth is sybil-attackable.** Smithery, the largest marketplace (100K+ tools), relies on GitHub account verification. Accounts are trivially created.
- **Post-publish attacks are the real threat.** Chrome Web Store's review-at-submission model allowed a 7-year campaign infecting 4.3M users. MCP "rug pull" attacks change tool behavior after gaining trust.
- **Economic incentives can be gamed.** tea.xyz rewarded package registration, leading to 150K+ spam packages flooding npm to farm tokens.

The official MCP Registry (registry.modelcontextprotocol.io) explicitly delegates curation to downstream aggregators. The Agent Skills spec (agentskills.io) deliberately defines no distribution or trust mechanism. There is an architectural gap exactly where a trust layer should sit.

## Why Kleros Curate TCR

Kleros Curate addresses the trilemma that all existing registries fail at:

| Property | npm/ClawHub | Chrome Web Store / Docker | Kleros Curate TCR |
|---|---|---|---|
| Permissionless submission | Yes | No | Yes |
| Ongoing monitoring | No | No (review at submit only) | Yes (challenge anytime) |
| Scales without bottleneck | Yes (but no review) | No (centralized reviewer) | Yes (distributed challengers) |
| Economic skin-in-the-game | No | No | Yes (deposits) |
| Anti-rug-pull | No | No | Yes (post-publish challenges) |

Key advantages over tea.xyz's failure: Kleros rewards *challengers who find bad entries*, not submitters. The economic incentive is for curation quality, not volume.

### How Kleros Curate Works

1. **Submission**: Submitter posts a deposit (in xDAI on Gnosis Chain) + entry metadata
2. **Challenge period** (typically 7 days): Anyone can challenge by posting their own deposit + evidence
3. **If unchallenged**: Entry is automatically accepted, deposit refunded
4. **If challenged**: Goes to Kleros Court — 3 randomly selected jurors evaluate evidence, vote independently. Loser forfeits deposit to winner.
5. **Appeals**: Anyone can appeal. Each round doubles jurors + 1 (3 → 7 → 15 → 31). Appeal fees can be crowdfunded.
6. **Ongoing**: Listed entries can be challenged for removal at any time (not just during initial review)

### Kleros Technical Details

- **Chain**: Gnosis Chain (low gas, xDAI stablecoin for predictable deposit costs)
- **Contract architecture**: Light Curate (item data on IPFS, only URI on-chain, ~700K gas deployment)
- **Querying**: Subgraph via The Graph (`litems` entity for Light Curate items)
- **Evidence standard**: ERC-1497 (on-chain events, IPFS content)
- **SDK**: `@kleros/gtcr-sdk` — `GTCRFactory`, `GeneralizedTCR` classes
- **Court**: Gnosis Curation Court (min stake 1,400 PNK, juror fee 7.2 xDAI, alpha 0.48)
- **Factory contracts**: Allow deploying separate registries per entry type with different deposit amounts
- **Supported chains**: 13 total (Ethereum, Arbitrum, Gnosis, Base, Polygon, zkSync, etc.)
- **Scale**: 200K+ tagged addresses, 508 disputes, 34% of Gnosis PNK in Curation Court
- **Incentives**: Scout program provides PNK reward pools per registry

## Entry Types

Three categories, tiered by risk:

| Category | Format | Risk | Deposit Tier | Description |
|---|---|---|---|---|
| **Conventions** | AGENTS.md, .cursorrules, CLAUDE.md | Low | Low | Project instructions, style rules, coding guidelines |
| **Skills** | SKILL.md (Agent Skills standard) | Medium | Medium | Instructions + optional scripts/templates. Cross-runtime (30+ tools). |
| **MCP Servers** | JSON config + source | High | High | Code-executing tool servers. Can access filesystem, network, shell. |

Plugins (bundles of skills + MCP + hooks) map to the highest-risk component they contain.

## Entry Metadata Schema

Each submission requires:

- **name**: Human-readable name
- **description**: What the entry does
- **author**: Ethereum address of the submitter
- **category**: `convention` | `skill` | `mcp_server` | `plugin`
- **target_runtimes**: Array of supported runtimes (e.g., `["claude_code", "openclaw", "cursor", "intendant", "codex", "generic"]`)
- **source_repo**: Git repository URL
- **commit_hash**: Full SHA-1 commit hash (pinned, immutable reference). This is the security model Git and GitHub rely on (SHA-1dc collision detection).
- **ipfs_cid**: IPFS CID of the content pinned at submission time (persistence even if repo deleted)
- **version**: Semver
- **declared_permissions**: What the entry needs access to (e.g., `["file_read", "file_write", "command_exec", "network"]`)
- **dependencies**: Other entries or system requirements

## Listing Criteria (Acceptance Policy)

This is the "Primary Document" that Kleros jurors evaluate against:

1. Content matches its stated description
2. No malicious behavior (data exfiltration, prompt injection in tool descriptions, credential theft)
3. Actually works with the declared target runtime(s)
4. Source code is available at the declared repo + commit hash
5. IPFS-pinned content matches the source at the declared commit
6. No known unpatched vulnerabilities
7. Does not impersonate another project (name squatting, typosquatting)
8. Declared permissions accurately reflect actual behavior
9. Dependencies are declared

## Trust Badges

Entries progress through trust levels:

- **Pending** — In challenge period (e.g., 7 days)
- **Listed** — Survived challenge period without successful challenge
- **Long-standing** — Listed for 6+ months without successful challenge
- **Audited** — Passed a formal third-party security audit (optional, premium tier)

## Version Lifecycle

- New version of a listed entry = new submission referencing the previous version
- Old verified versions remain listed
- Users choose: latest Intendhub-verified version, or pin to a specific verified version
- If the source repo changes (potential rug pull), anyone can challenge the listing

## Automated Pre-Screening

Automated scanning supplements (not replaces) the TCR:

- Run `mcp-scan` (Invariant Labs' tool poisoning scanner) on MCP server submissions
- Static analysis for known malicious patterns
- Prompt injection detection in tool descriptions and skill instructions
- Dependency scanning
- Results published as public evidence that challengers can reference
- **Not a gatekeeper** — results are informational, the TCR decides

## Positioning: Trust Layer, Not Distribution

Intendhub does NOT replace existing distribution channels. It complements them:

- **Smithery, ClawHub, skills.sh, npm** = discovery and distribution (find and install stuff)
- **Intendhub** = trust verification (verify stuff is safe)

An Intendhub listing says: "This specific version of this entry, at this commit hash, has been community-verified as non-malicious and compliant with listing criteria."

Entries can link to their distribution points on other platforms. Runtimes query the Intendhub subgraph to check if an entry is verified before installing.

## Architecture

```
Gnosis Chain
  └── Kleros Curate Light Curate contracts
      ├── Skills Registry (medium deposits)
      ├── MCP Servers Registry (high deposits)
      └── Conventions Registry (low deposits)

IPFS
  └── Pinned content snapshots at submission time
  └── Evidence documents for disputes

The Graph
  └── Subgraph indexing all three registries
      └── Queryable by CLI, frontend, and runtime integrations

Frontend (Web)
  └── Browse, search, submit, challenge
  └── Per-runtime filtering
  └── Trust badge display
  └── Dispute evidence viewer

CLI (intendhub)
  └── intendhub search "github" --runtime intendant
  └── intendhub install <entry-id>
      ├── Generates intendant.toml [[mcp_servers]] entry
      ├── Or drops SKILL.md into .claude/skills/ or .intendant/skills/
      ├── Or generates .cursorrules / AGENTS.md
      └── Pins to the verified commit hash
  └── intendhub verify <repo-url> <commit-hash>
  └── intendhub submit <entry>
  └── intendhub challenge <entry-id> --evidence <file>

Automated Scanner
  └── Runs on new submissions
  └── Publishes results to IPFS as evidence
  └── Does NOT gate submissions — informational only
```

## Tech Stack

- **Smart contracts**: Solidity (Kleros Curate Light Curate, possibly with thin wrapper contracts)
- **Subgraph**: AssemblyScript (The Graph)
- **Frontend**: TypeScript, React (or similar), ethers.js/viem for wallet interaction
- **CLI**: TypeScript/Node (aligns with MCP/npm ecosystem)
- **IPFS pinning**: Pinata, nft.storage, or self-hosted

## Current Agent Ecosystem Context (March 2026)

### Standards
- **SKILL.md**: Cross-runtime agent skills standard. Adopted by 30+ tools (Claude Code, Codex, Cursor, Copilot, Gemini CLI, etc.). YAML frontmatter + markdown instructions. Progressive disclosure model.
- **MCP**: Model Context Protocol. 97M monthly SDK downloads. 10,000+ servers. Under Linux Foundation's Agentic AI Foundation (AAIF). OAuth 2.1 for auth.
- **AGENTS.md**: Cross-tool project instructions standard. 60,000+ repos. Under AAIF.

### Major Runtimes
- Claude Code (Anthropic) — 41% professional developer usage. Plugin marketplace.
- OpenAI Codex CLI — 65K+ stars. Plugin system launched March 26, 2026.
- Gemini CLI (Google) — 96K+ stars.
- OpenClaw — 250K+ stars. Open source. ClawHub registry (13,729 skills, heavily compromised).
- Cursor — 1M+ users. Marketplace launched Feb 2026.
- Goose (Block) — Open source, MCP-native.
- Devin (Cognition) — Most autonomous, sandboxed cloud.

### Existing Registries
- **Official MCP Registry** (registry.modelcontextprotocol.io): Metaregistry, ~2,000 entries, delegates curation
- **Smithery**: 100K+ tools/skills, CLI-based, centralized, GitHub-auth trust
- **skills.sh** (Vercel): 4,257+ skills, Snyk scanning partnership
- **SkillsMP**: 66,541+ skills, GitHub scraper, minimal verification
- **ClawHub**: 13,729 skills, 36.8% had security flaws
- **Claude Code marketplace**: Official + community marketplaces, managed restrictions
- **Docker MCP Catalog**: 300 servers, signed images, strongest trust but doesn't scale

### Security Landscape
- OWASP MCP Top 10 published
- 30 CVEs in 60 days for MCP
- 82% of 2,614 MCP implementations have path traversal vulnerabilities
- 66% of 1,808 MCP servers scanned by AgentSeal had security findings
- Tool poisoning: invisible to static analysis (lives in tool descriptions, not code)
- Tool shadowing: malicious tool alters behavior of OTHER tools on different servers
- Rug pulls: server changes behavior after gaining trust
- Supply chain: npm debug/chalk compromised, Shai-Hulud worm hit 180+ packages

### Adjacent Projects
- **MCP Secure** (mcp-secure.dev): Cryptographic identity/signing for MCP
- **MCPTrust**: Runtime security proxy, lockfile enforcement, drift detection
- **tea.xyz**: Blockchain package curation on Base — failed due to incentive design (rewarded volume, not quality)
- **Solana Agent Registry**: On-chain agent identity
- **ERC-8004**: Ethereum agent identity/reputation standard

## Open Questions

1. **Deposit amounts**: What are the right deposit tiers for conventions/skills/MCP servers on Gnosis? Need to be high enough to deter spam but low enough for indie developers.
2. **Specialized court**: Should we propose a dedicated "Agent Security Court" in Kleros, or use the existing Curation Court?
3. **IPFS pinning economics**: Who pays for persistent pinning? Submitter? Protocol treasury?
4. **Scanner integration**: Build our own automated scanner, or integrate existing tools (mcp-scan, Snyk)?
5. **CLI distribution**: npm package? Standalone binary? Both?
6. **Frontend hosting**: IPFS (fully decentralized) vs Vercel (better UX, centralized)?
7. **Incentive bootstrapping**: Run a Scout-style incentive program to bootstrap initial submissions?
8. **Cross-registry linking**: How to formally link an Intendhub verification to a Smithery/ClawHub entry?
