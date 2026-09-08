---
name: intendancy
description: "Use when an agent needs skills from the Agent Skills Registry, the Kleros-curated skill list on Gnosis. DOWNLOADS AND RUNS AN EXTERNAL BINARY: the intend CLI v0.1.0-alpha.1 from the github.com/lovon-spec/Intendancy release, executable digests linux-x86_64 sha256:5682e412a647a6cd05085c4dda3f6462152892a699da51ac761ee4c080748e45, linux-aarch64 sha256:2fe1ea4d61b5dd69c9541d3b46d2e75c23f835e8dfd940af7b56d0d6369db131, macos-x86_64 sha256:6ba45797c4899abd2ef0c60f4592ad27317596e34a65da6aa28f28dd358efd3b, macos-aarch64 sha256:a922991eb8d53124fee9fef82fe8dd9027de73091b3fb8eb6c6e71615505f4ac, profile sha256:805c24364d9caa4994601216decb78c6ef8793cf0bb2a254ce43eb7abb4e9bfb. The installer verifies release signature, archive and binary digests and aborts on mismatch before anything runs. Then: verified catalog refresh, skill lookup, exact-bytes install of separately registered skills, audit. Verification is relative to a header-quorum anchor (alpha); a strict light client is pending."
license: MIT
compatibility: "Requires network access and a POSIX shell with curl, tar, gpg 2.x, and sha256sum or shasum. Installs the intend binary and the registry profile under $INTEND_HOME (default ~/.intend). Linux and macOS, x86_64 and aarch64."
---

# Intendancy: skills from the Agent Skills Registry

The Agent Skills Registry is a Kleros Curate list on Gnosis Chain. Anyone may list a skill by posting a deposit; anyone may challenge it; challenged entries are decided by jurors against the listing policy. An entry that survives is Registered. For each entry the registry stores the content address (CID) of the exact skill tree. The `intend` CLI turns that into an install path: it proves the complete list against chain state anchored by a header quorum, fetches the tree from public IPFS gateways, verifies every block by hash, and installs exactly those bytes.

What this skill downloads and runs is fixed: the intend CLI release named in the description, whose executables are identified there by digest; `scripts/install-intend.sh` also pins each platform archive and the profile. The installer verifies the release key against a pinned fingerprint, requires every signature to be made under that key, checks the archive, the extracted executable and the profile against the pinned digests, and installs nothing on any failure. A newer CLI ships as a new version of this skill with new digests; this version does not update itself.

The CLI treats the RPC, the gateways and any snapshot provider as untrusted and fails closed. Its anchor today is a finalized header agreed by independent public RPCs, which the tool labels a degraded alpha; a strict light client is pending. Report that mode whenever you report on installed skills.

## 1. Install the CLI, verified

From the root of this skill tree:

```sh
sh scripts/install-intend.sh
```

Installs to `$INTEND_HOME` (default `~/.intend`): `bin/intend`, `profiles/agent-skills-registry.toml` (the registry address, code hash, arbitrator, governor and policy references, all pinned) and `state/`. Pass `--home DIR` or set `INTEND_HOME` to choose the location; keep using the same one below. If the installer reports any mismatch, stop and do not install by other means.

Define the command once, with the same home and one persistent lockfile:

```sh
INTEND_HOME="${INTEND_HOME:-$HOME/.intend}"
intend_cli() { "$INTEND_HOME/bin/intend" --profile "$INTEND_HOME/profiles/agent-skills-registry.toml" --state-dir "$INTEND_HOME/state" "$@"; }
LOCK="$INTEND_HOME/state/intend-lock.json"
```

## 2. Refresh the catalog

```sh
intend_cli update
```

Prints one JSON line. Before trusting the catalog check three fields: `ok` is `true`; `anchorMode` names the anchor, `header-quorum` today, labelled a degraded alpha; `enumeration` says `complete`, the registry's full item list was proven, not sampled. `anchorAgeSecs` is the age of the anchored block; every install performs a fresh check regardless.

## 3. Find a skill

```sh
intend_cli catalog
```

Lists every entry with its status, item ID, name and tree CID. Filter by name with `grep`. Only `Registered` entries are installable; `RegistrationRequested` means the entry is still in its challenge period; `ClearingRequested` means a removal is pending and the entry is suspended; `Absent` means rejected or removed.

## 4. Install a skill

```sh
intend_cli install <name-or-0xitemID> --dir "<skills-directory>/<name>" --lockfile "$LOCK"
```

The directory receives exactly the bytes the registry points at, after a fresh on-chain status check and full hash verification of the tree; the lockfile records the registry, the item ID, the tree CID and the block the entry was verified at. Where skills live depends on the runtime: Claude Code loads `~/.claude/skills/<name>/` for the user and `.claude/skills/<name>/` for a project; for another runtime use its documented skills directory. Never copy a tree from a gateway or a repository by hand: the install path is the verification.

## 5. Audit installed skills

```sh
intend_cli audit --lockfile "$LOCK"
```

Re-checks every lockfile entry against the registry's current state and records the result. Audit reports and records; it does not stop a runtime from loading a directory, and `enable` verifies a skill at the path recorded in the lockfile, so a moved tree cannot be re-enabled where it sits. Use this sequence for an entry reported as suspended (`ClearingRequested`) or revoked (`Absent`):

1. Move its directory out of the skills path, for example to `$INTEND_HOME/quarantine/<name>`, keeping the bytes and the lockfile for inspection. Do not edit the tree.
2. To re-enable later: stop every runtime that discovers that skills directory, move the directory back to its recorded path, run `intend_cli enable "<recorded-path>" --lockfile "$LOCK"`, and if that fails move it out again before any runtime resumes. Only a successful `enable` on a fresh status and integrity check leaves the skill in place.

## Rules

- Use only a profile obtained through this installer's verification or another signed release.
- Do not bypass a failed check by downloading binaries, profiles or trees elsewhere.
- Say which anchor mode the catalog was verified under whenever you report on installed skills.
