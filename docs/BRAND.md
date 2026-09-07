# Intendancy naming and brand guide

This document is the canonical naming reference for this repository.

## Canonical names

| Name | Use | Do not use as a substitute |
|---|---|---|
| **Intendancy** | The consumer product: the `intend` distribution, the website and the registry profile. Never the on-chain registry itself. | Intendhub, marketplace, app store, the name of the list. |
| **Agent Skills Registry** | The on-chain Kleros Curate list, deployed as unbranded neutral infrastructure. | Intendancy Registry, Intendancy Skills Registry. |
| **`intend`** | The generic verifier, package-management machinery, CLI command, and binary. | Intendancy CLI, unless discussing the product distribution as a whole. |
| **Intendant** | The first consuming agent runtime. | Intendancy. |
| **Intendment** | The separate optimistic-dispute settlement project. | A component of Intendancy or Intendant. |

Capitalize **Intendancy**, **Intendant**, and **Intendment** when naming products. Use lowercase **`intend`** in code formatting for the command, binary, crate, paths, and machine-readable identifiers that intentionally belong to the generic verifier.

## Positioning

Preferred one-line description:

> **Intendancy is the verifiable, policy-governed way to use the Agent Skills Registry.**

Expanded description:

> Intendancy gives Intendant and other agent runtimes a package catalog whose membership and artifact integrity can be checked without trusting a marketplace backend, CDN, IPFS gateway, indexer, or execution RPC.

Relationship statement:

> **`intend` verifies and installs. Intendancy governs the catalog. Intendant consumes it.**

The product is a **registry**, **catalog**, **governed capability domain**, or **registry profile** depending on context. It may provide a web interface, but it is not named or positioned as a “hub.” Avoid calling it a marketplace: submissions and challenges can be transacted through the frontend, but the core product claim is independently verifiable curation and exact artifact installation, not trusted commercial intermediation.

## The registry is unbranded

Owner decision, 2026-09-04. The on-chain registry is deployed as the **Agent Skills Registry**: its MetaEvidence titles, list title, logo, listing policy and juror evidence display carry no Intendancy branding, so that it reads as shared infrastructure in Kleros's list of lists and any runtime or submitter can build on it. Intendancy brands only the consumer side. The registry's logo (`meta-evidence/agent-skills-registry-logo.svg`) is therefore the name itself: the words "Agent Skills" stacked on a white plate, "Agent" in near-black and "Skills" in teal, set in Inter (SIL Open Font License 1.1) and outlined to paths by `tools/logo/build-logo.py`, so that it carries no mark that a later, closer integration with Intendancy, Intendment or Intendant would have to explain away. The teal sits apart from the Kleros purple and from the Intendancy green. When describing where governance is headed, say "the registry's community"; do not name any single organization as its future steward.

## Naming rationale

An **intendancy** is the office, function, body, or administrative domain of an intendant. The relationship is deliberately close:

- an **Intendant** is the supervised runtime that acts;
- an **Intendancy** is the governed domain of capabilities from which it may draw; and
- the neutral **`intend`** machinery verifies that domain and materializes exact artifacts.

This is a product pair, not three artificially equal siblings. **Intendment** remains linguistically related but architecturally separate.

## Visual identity

Intendancy is a visual sibling of Intendant, not a recolored clone. The two marks share the visual grammar of the governed house:

- a charcoal plate that works on light and dark surfaces;
- a graphite proscenium arch;
- a sky-blue serif **I**;
- warm gold for delegated authority and policy; and
- green for verified, admissible state.

The distinction carries the product boundary. **Intendant** crosses the I with a conductor’s baton and fans outward to orchestrated agents. **Intendancy** crosses the same I with horizontal registry-ledger rows terminating in verified-status nodes. One runs the house; the other represents its governed capability domain.

`assets/intendancy-mark.svg` is the canonical master. `frontend/public/favicon.svg` must remain a byte-identical copy until a build step replaces that duplication; the on-chain registry's logo is the separate, unbranded wordmark described above. The brand check enforces this invariant.

## Technical naming rules

1. Rename product-facing `intendhub` identifiers to `intendancy`, including package names, profile names, test types, fixture labels, frontend titles, documentation, and repository-local URLs.
2. Keep the `intend` CLI, binary, crate family, command examples, lockfile format terminology, and generic materializer concepts named `intend`.
3. Do not rename protocol-standard or upstream identifiers such as `GeneralizedTCR`, ERC interfaces, Kleros names, Gnosis names, IPFS/CAR/UnixFS terms, or third-party package names.
4. Treat deployed addresses, immutable CIDs, published hashes, and historical external references as data rather than branding. Never rewrite them merely to make the spelling look current.
5. The two listing-policy copies—`docs/listing-policy.md` and `frontend/public/listing-policy.md`—must remain byte-identical.

## Repository and package slugs

The intended repository slug is `lovon-spec/intendancy`. GitHub redirects old clone and web URLs after a repository rename, but documentation and package metadata should use the new slug once the rename is complete.

Preferred package identifiers:

- frontend package: `intendancy-frontend`
- product profile IDs: `intendancy-*`
- generic Rust package and binary identifiers: retain `intend-*` or `intend`

## Deprecated name

**Intendhub** and **intendhub** are deprecated. They may appear only when quoting immutable historical material or documenting the migration itself. New prose, code, filenames, package metadata, screenshots, and UI labels must use **Intendancy**.
