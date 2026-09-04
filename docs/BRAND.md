# Intendancy naming and brand guide

This document is the canonical naming reference for this repository.

## Canonical names

| Name | Use | Do not use as a substitute |
|---|---|---|
| **Intendancy** | The product and concrete governed skill-registry profile. | Intendhub, marketplace, app store. |
| **`intend`** | The generic verifier, package-management machinery, CLI command, and binary. | Intendancy CLI, unless discussing the product distribution as a whole. |
| **Intendant** | The first consuming agent runtime. | Intendancy. |
| **Intendment** | The separate optimistic-dispute settlement project. | A component of Intendancy or Intendant. |

Capitalize **Intendancy**, **Intendant**, and **Intendment** when naming products. Use lowercase **`intend`** in code formatting for the command, binary, crate, paths, and machine-readable identifiers that intentionally belong to the generic verifier.

## Positioning

Preferred one-line description:

> **Intendancy is a verifiable, policy-governed registry for agent skills.**

Expanded description:

> Intendancy gives Intendant and other agent runtimes a package catalog whose membership and artifact integrity can be checked without trusting a marketplace backend, CDN, IPFS gateway, indexer, or execution RPC.

Relationship statement:

> **`intend` verifies and installs. Intendancy governs the catalog. Intendant consumes it.**

The product is a **registry**, **catalog**, **governed capability domain**, or **registry profile** depending on context. It may provide a web interface, but it is not named or positioned as a “hub.” Avoid calling it a marketplace: submissions and challenges can be transacted through the frontend, but the core product claim is independently verifiable curation and exact artifact installation, not trusted commercial intermediation.

## Naming rationale

An **intendancy** is the office, function, body, or administrative domain of an intendant. The relationship is deliberately close:

- an **Intendant** is the supervised runtime that acts;
- an **Intendancy** is the governed domain of capabilities from which it may draw; and
- the neutral **`intend`** machinery verifies that domain and materializes exact artifacts.

This is a product pair, not three artificially equal siblings. **Intendment** remains linguistically related but architecturally separate.

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