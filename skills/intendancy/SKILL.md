---
name: intendancy
description: Use when an agent needs skills from the Agent Skills Registry, the Kleros-curated skill list on Gnosis Chain. Installs the intend CLI from a signed release with every artifact verified, refreshes the locally verified catalog, finds a skill, installs its exact bytes into the runtime's skills directory, and audits what is installed. Trust comes from finalized chain state and content hashes, never from a server.
compatibility: Requires network access and a POSIX shell with curl, tar, gpg, and sha256sum or shasum. Installs the intend binary and the registry profile under $INTEND_HOME (default ~/.intend). Linux and macOS, x86_64 and aarch64.
---

# Intendancy: skills from the Agent Skills Registry

The Agent Skills Registry is a Kleros Curate list on Gnosis Chain. Anyone may list a skill by posting a deposit; anyone may challenge it; challenged entries are decided by jurors against the listing policy. An entry that survives is Registered. The registry stores, for each entry, the content address (CID) of the exact skill tree. The `intend` CLI turns that into an install path an agent can rely on: it proves the complete list against finalized chain state, fetches the tree from public IPFS gateways, verifies every block by hash, and installs exactly those bytes.

The CLI treats every server as untrusted: the RPC, the gateways, and any snapshot provider. If a check fails, it fails closed and says which check.

## 1. Install the CLI, verified

Run the installer that ships with this skill:

```sh
sh scripts/install-intend.sh
```

It downloads the newest release, checks the release signing key against the fingerprint pinned in the script, verifies the signed checksum list, the archive and the registry profile, and installs to `$INTEND_HOME` (default `~/.intend`):

- `~/.intend/bin/intend`, the binary
- `~/.intend/profiles/agent-skills-registry.toml`, the profile: the registry address, its code hash, arbitrator, governor and policy references, all pinned
- `~/.intend/state/`, the verified catalog and journal

Pin a version with `--version vX.Y.Z`. If the installer reports a fingerprint or signature mismatch, stop; do not install by other means.

Set, for the rest of this skill:

```sh
INTEND="$HOME/.intend/bin/intend --profile $HOME/.intend/profiles/agent-skills-registry.toml --state-dir $HOME/.intend/state"
```

## 2. Refresh the catalog

```sh
$INTEND update
```

The command prints one JSON line. Read three fields before trusting the catalog:

- `ok` must be `true`.
- `anchorMode` says how the chain state was anchored. `header-quorum` is the current mode; it is labelled a degraded alpha because a strict light client is still pending. Report the mode, do not hide it.
- `enumeration` must say `complete`: the registry's full item list was proven, not sampled.

`anchorAgeSecs` is the age of the anchored block. Every install performs a fresh check regardless of catalog age.

## 3. Find a skill

```sh
$INTEND catalog
```

Lists every entry with its status, item ID, name and tree CID. Filter by name with `grep`. Only entries in status `Registered` are installable; `RegistrationRequested` means the entry is still in its challenge period; `ClearingRequested` means a removal is pending and the CLI suspends the skill; `Absent` means rejected or removed.

## 4. Install a skill

```sh
$INTEND install <name-or-0xitemID> --dir <skills-directory>/<name>
```

The directory receives exactly the bytes the registry points at, after a fresh on-chain status check and full hash verification of the tree. The lockfile `intend-lock.json` in the current directory records the registry, the item ID, the tree CID and the block the entry was verified at.

Where skills live depends on the runtime. Claude Code loads `~/.claude/skills/<name>/` for the user and `.claude/skills/<name>/` for a project. For another runtime, use its documented skills directory. Never copy a tree from a gateway or a repository by hand: the install path is the verification.

## 5. Audit installed skills

```sh
$INTEND audit
```

Re-checks every lockfile entry against the registry's current state. A skill whose entry is now `ClearingRequested` or `Absent` is reported as suspended or revoked; leave it disabled until `$INTEND enable <dir>` succeeds after a fresh proof.

## Rules

- Do not use a profile you did not obtain through the installer's verification or another signed release.
- Do not bypass a failed check by downloading trees or binaries elsewhere.
- Say which anchor mode the catalog was verified under whenever you report on installed skills.
