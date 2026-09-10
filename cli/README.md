# intend

Intendancy consumer CLI implementing
[`docs/verified-snapshot-spec.md`](../docs/verified-snapshot-spec.md) v0.2 —
the frozen proof core plus the PROVISIONAL §8/§9 operational contract:
complete verified registry snapshots (`update`), exact skill installation with
full DAG verification (`install`), and lockfile auditing with quarantine and
lockfile-local revocation (`audit`).

**Pre-1.0 status, reported honestly on every command** (anchor strength,
enumeration scope, and freshness are separate outputs, never one "verified"
boolean):

- **Anchor**: header-quorum only (≥2 independent RPCs must agree on the
  finalized header) — the labeled DEGRADED ALPHA mode. Integrating the Gate 1
  embedded consensus light client as the strict mode is tracked work; only that
  mode earns "trustless".
- **Transport**: the spec's JSON debug encoding (raw or gzip, byte-capped before
  materialization). Binary framing lands with the spec's framing spike.
- **Install profile**: bounded **UnixFS-basic** — a deliberately NARROW,
  provisional **kubo-interop profile, not general UnixFS/dag-pb conformance**:
  plain Directory nodes, single-block raw leaves ≤ 256 KiB, and kubo-style
  single-level chunked File nodes, **proven byte-compatible with kubo's default
  `ipfs add --cid-version 1` output in BOTH directions** by the committed
  vectors in `fixtures/kubo/` (consumer: kubo's CARs — default AND the
  metadata-bearing mode+mtime+exec-bit vector — parse, verify, and install
  byte-identically; producer: our builder reproduces kubo's exact root CID,
  chunked 600 KB file included). Mode/mtime metadata is accepted at most once
  each and ignored (mtime's embedded message is length-skipped, not
  validated); the UnixFS Type field is required first; HAMT-sharded
  directories and multi-level File trees stay outside the profile (impossible
  under the 2 MiB policy cap with default chunking) and fail closed.

## Releases

Releases are cut from SSH-signed `v*` tags by `.github/workflows/release.yml`. Every asset is PGP-signed with the same key that signs Intendant's releases; its public half is committed at the repository root as `RELEASE-SIGNING-KEY.asc` and is uploaded beside the assets, and the workflow verifies each signature against that committed key before publishing.

Assets: `intend-<version>-<target>.tar.gz` for linux-x86_64, linux-aarch64, macos-aarch64 and macos-x86_64 (the binary plus this README), `agent-skills-registry.toml` (the production profile), `SHA256SUMS`, and one detached `.asc` signature per file.

Verify before use, the profile above all: it is the deployment manifest the CLI trusts.

```bash
gpg --import RELEASE-SIGNING-KEY.asc
gpg --verify SHA256SUMS.asc SHA256SUMS
sha256sum -c --ignore-missing SHA256SUMS
gpg --verify agent-skills-registry.toml.asc agent-skills-registry.toml
```

## Bootstrapping an agent

`skills/intendancy/` at the repository root is a skill that teaches an agent to use the registry: `scripts/install-intend.sh` installs the CLI and the profile from a signed release with every artifact verified against the pinned release key, and `SKILL.md` covers update, catalog, install and audit, including how to read the anchor mode. An agent gets its first copy from this repository; once the skill is listed in the registry, updates arrive through the same verified path as every other skill.

## Verified exports of Curate lists

`curate-export`, a second binary in this crate, snapshots a Kleros **Light** Curate list from the chain itself, so that curators and integrators can rely on a file they can rerun and compare instead of an indexer or a published export. The four Kleros Scout registries on Gnosis are presets (`--registry tokens|address-tags|atq|cdn`); any other Light list works with `--list`, `--items-slot` and `--chain-id`/`--genesis`.

What it proves, and what it does not:

- **Membership and status are proven.** A Light list keeps `items[itemID]` in storage; the export proves every candidate's status slot with `eth_getProof` at a header-quorum anchor, verified against the anchor's state root by the same MPT verifier `intend` uses.
- **Content is bound.** `itemID` is the keccak of the item's IPFS path, and the item file is fetched as IPFS blocks whose bytes must hash to the CID, from the gateways in order.
- **Completeness rests on logs.** A Light list has no item list in storage; candidates come from `NewItem` logs, so the export takes them from at least two independently operated RPCs and requires the sets to agree. An item both sources hide is invisible. The provenance file says so, and the anchor is the header quorum this crate labels an alpha mode.

Sources, as verified on 2026-09-09: the Gnosis foundation RPC and Tenderly's public gateway both serve `NewItem` logs over five-million-block windows, so they are the default log sources (two operators); PublicNode and dRPC cap a logs request at 10,000 blocks and work too, at two thousand calls per registry, and the export shrinks its window to whatever limit a source names; 1rpc allows 50 blocks and is unusable for this. The foundation RPC serves proofs only near the head, so PublicNode remains the proof provider, and the two together are the header quorum. Gateways: every block is hash-verified, so any gateway is safe to read from; trustless-gateway.net, Kleros's own CDN and Filebase's gateway answer path-style raw-block requests directly and are the defaults, while dweb.link and ipfs.io redirect twice per block; the exporter tries gateways healthiest-first within a run, so one stalled gateway does not cost every item its deadline.

```sh
cargo build --release --bin curate-export
B=target/release/curate-export

# 1. A registry-agnostic snapshot: items.json (+ CSV), provenance.json.
$B export --registry address-tags --out items.json --provenance provenance.json --csv items.csv
$B export --registry tokens --include-pending    # also the entries under review, for challengers

# 2. Offline queries over the snapshot: is this address already tagged? which entries mention this domain?
$B lookup --items items.json --address 0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48
$B lookup --items items.json --value kleros.io --status 1

# 3. The Tokens list as a Uniswap-schema token list, with a diff against the discontinued export.
$B tokenlist --items tokens-items.json --compare https://t2crtokens.eth.limo/ --out tokens.json --diff-out diff.json
```

`items.json` carries, per item, the item id, the status (name and number), the IPFS path, and the item file's `columns` and `values` verbatim, sorted by item id; statuses Registered and ClearingRequested by default, `--include-pending` adds RegistrationRequested, `--all` adds Absent with only the path its log carried. `tokenlist` reads that snapshot (or runs the export itself when given the source flags): rich addresses become chain id and EIP-55 address, decimals are validated, logos become `ipfs://` URIs, the output is deterministic, `--previous` applies token-lists versioning and `--compare` writes a diff. `provenance.json` records the anchor, every source, the block range, the status histogram, every skipped item with its reason, the RPC call count and the sha256 of the output. `tools/smoke-curate-export.sh` runs all of it against the live registries.

To publish a snapshot or a token list: pin it with `tools/launch/car-of.sh --wrap` and `pin-filebase.sh`, or any pinning service, and hand out the CID with the provenance file. Anyone who distrusts the publisher reruns the command and compares.

## The production profile

Since 2026-09-09 the profile lists policy version 1 (listing policy 2.3) under `policy_updates`, so it accepts the registry at `metaEvidenceUpdates` 0 or 1 and fails closed on anything else. Listing a version changes the deployment context, so installs made under the alpha.1 profile are carried over with `intend migrate --from agent-skills-registry.toml` using the alpha.1 profile asset; a profile with any other trust field changed, or one that drops a version, is refused as a non-successor.



`profiles/agent-skills-registry.toml` is the deployment manifest for the Agent Skills Registry on Gnosis (registry `0x67DBE6A9597635074546e08B92eE617bF02168f9`, governor the timelock `0xc8Ba4c0AD3554EDB0a9A4C8D73Bf87410A313ADa`, court 19 with three jurors, the two MetaEvidence references the deployment emitted). It ships as a release asset and is verified before use. First verified against the live chain on 2026-09-07: header quorum from two operators, complete enumeration proven, an empty catalog.

Two facts about public Gnosis endpoints shape it. The foundation RPC (`rpc.gnosischain.com`) serves `eth_getProof` only about 32 blocks behind head, short of finalized depth, so the proof provider is PublicNode. And the header quorum takes the lowest finalized block across its sources and needs every source to answer, so a source with lagging finality (1rpc, about forty minutes behind) or intermittent errors (dRPC) is left out.

## Commands

```bash
intend --profile profile.toml update                 # snapshot_urls failover, else self-generate; verify; persist
intend --profile profile.toml update --snapshot s.json.gz   # verify a provider file instead
intend --profile profile.toml catalog                # list the verified catalog
intend --profile profile.toml install <name|0xitemID> --dir ./skill [--car tree.car]
intend --profile profile.toml audit [--lockfile intend-lock.json]
intend --profile profile.toml enable ./skill         # clear a sticky suspension (fresh proof + integrity required)
intend --profile new.toml migrate --from old.toml    # carry every entry across a policy-version transition
```

- `update` — quorum-anchors a finalized header, obtains a complete snapshot
  (self-generated via the untrusted provider RPC using Multicall3 batching, or
  a provider file/URL whose declared anchor is then quorum-authenticated),
  verifies spec §6 steps 0–7 (profile binding incl. proven codehash, complete
  enumeration, the POLICY VERSION — `metaEvidenceUpdates` proven to be a version
  the profile accepts (0, or one it lists with its announced references), the
  ARBITRATOR PIN — slot 0 and the `arbitratorExtraData` words proven equal to the
  profile's `arbitrator`/`arbitrator_extra_data`, and the GOVERNOR PIN — slot 3
  proven equal to the profile's `governor`, all at every anchor, so a governor's
  policy change, court switch or self-replacement fails closed until a new signed
  profile is installed — statuses in both zero-proof forms, descriptor
  screening), enforces the §4.1 resource limits and §8 freshness/rollback rules
  (versioned high-water file), and persists the catalog. Fresh point checks
  re-prove the policy version and both pins at THEIR anchor too.
- `install` — resolves the item, re-screens, does a FRESH point check at the
  latest finalized quorum anchor (only `Registered` installs; "verified,
  non-exhaustive" by construction). NAME-based installs additionally prove
  the resolution FRESH: the registry's current itemCount must equal the
  verified catalog's (descriptors are immutable, so count equality proves the
  same-name candidate set complete), and EVERY same-name candidate's status
  is freshly proven with exactly one allowed to be Registered — a name that
  is ambiguous NOW refuses even if the saved catalog was unambiguous
  (0x-itemID installs are the recommended exact path). It then fetches the CAR from a local file or the
  profile's gateways (byte-capped stream), verifies the complete DAG against
  the DESCRIPTOR's Tree CID (the CAR's own root claim is never the authority;
  every block rehashed), then runs the install TRANSACTION: bind the
  destination once (canonical parent held as a directory fd), check occupancy
  and the lockfile record BEFORE journaling (a repeat install fails with the
  prior record untouched), persist a PENDING journal entry carrying the FULL
  verified manifest, stage into a fresh 0700 directory and publish with one
  atomic NO-REPLACE rename through the bound fd (files, directories, and the
  parent fsynced — content durable before the name, the name durable before
  the record), and finalize by clearing the pending flag (itemID, Tree CID,
  binding, status, anchor block number AND hash, per-file sha256). The bound
  parent's IDENTITY (dev/ino vs the held fd) is spot-checked before publish,
  before the publish rename, and before finalization — a retargeted pathname
  fails closed with the journal left pending, and error cleanup is
  fd-relative, so a stale path is never knowingly finalized or resolved again.
- `audit` — verifies the INSTALLED BYTES against the lockfile (exact file set,
  sizes, digests; divergence is the fail-closed `modified` state), then a fresh
  point check per entry — a check that FAILS for one entry records
  "check-failed" for it and keeps sweeping, so sticky transitions observed on
  other entries are always persisted (the command still exits 1): `current`
  (Registered, intact), `quarantined`
  (ClearingRequested — requester-neutral suspension), `revoked` (this exact
  item was verified Registered locally and is now Absent — the lockfile-local
  rule; unrelated Absent rows never become a revocation list), `blocked`
  otherwise. Quarantine and revocation are STICKY: a later return to Registered
  reports `reenable-required` until the explicit `enable` transition (fresh
  Registered proof + local integrity) clears it — and stickiness is acquired
  from the PROVEN status even while the tree is locally `modified`. Audits also
  verify the exact DIRECTORY set (empty-dir drift detected), refuse symlinked
  install roots, non-UTF-8 names, and special node kinds, and check sizes from
  metadata before bounded streaming hashes. (Scope, honestly: these checks are
  point-in-time, not race-free against a CONCURRENTLY modifying local attacker
  — a named spec-§9 production blocker.) Interrupted install transactions
  (pending journal entries) audit as fail-closed `incomplete` — while still
  ACQUIRING sticky suspension from the proven status — until `enable`
  finalizes them against the journaled manifest after a fresh Registered
  proof + integrity pass, re-binding the destination and RE-FSYNCING its
  parent first (a crash between the publish rename and the parent fsync
  leaves visible content whose dirent is not yet durable; the finalizer
  closes that window before clearing `pending`). Recovery scope, split by boundary: (1) a
  crash before the journal's atomic rename leaves no record and no content —
  ordinary retry; (2) a crash after the journal is durable but before publish
  leaves a fail-closed pending journal whose removal (to reinstall at that
  path) is a deliberate manual lockfile edit, and a crash during staged
  extraction may additionally orphan one `.intend-stage-*` directory (safe to
  delete when no installer is running); (3) a crash after the publish rename
  or during the finalizing save recovers via audit + enable. Non-current
  states exit 1.
- `install` additionally enforces the deterministic policy binding: the tree
  must carry a root `SKILL.md` whose frontmatter `name`/`description`
  byte-match the descriptor columns; gateway failover continues past
  verification failures, not just HTTP errors.

## Policy version transitions

A profile that accepts a newly announced policy version (one more
`[[policy_updates]]` entry) has a different deployment context id, because the
accepted policy set is part of the trust context every catalog and lockfile
entry is bound to. Installing the new profile therefore makes `audit` and
`enable` refuse every entry installed under the previous one, by design: state
verified under one policy set is never consumed under another silently.

`intend --profile new.toml migrate --from old.toml` carries the entries across
explicitly. It requires the new profile to be a strict successor of the old
one — the same chain, genesis, registry, code hash, arbitrator and extra data,
governor and deployment MetaEvidence pins, and the old profile's accepted
versions as a prefix of the new one's, with at least one version added;
anything else is a different deployment or a different trust decision and is
refused with the field named. Every entry must be bound to the old context
(entries already at the new context are skipped). Each entry then gets a local
integrity pass and a fresh point check under the NEW profile — status,
accepted policy version, governor, arbitrator and code hash at one
authenticated anchor — before it is rebound. Sticky suspensions are preserved
or acquired, never cleared (only `enable` clears them); a pending journal stays
pending; a migration record (old and new context, anchor, status, state, the
local-integrity verdict, and the audit record the entry carried before) is
appended to the entry. Context rebinding is all-or-nothing.

Two things do not stop a migration. An entry whose tree is missing, moved
(the bootstrap skill's quarantine procedure moves a suspended tree out of its
discovery directory and keeps the lockfile entry) or modified, or whose
install never finished, migrates in that recorded state — `modified` or
`incomplete`, with the verifier's text in the record — its manifest, history
and sticky restrictions intact and its bytes never reported intact or
enabled; `enable` still requires the tree at the recorded path. And a point
check that fails for a later entry aborts the rebinding of every entry but
does not lose what was already proven: adverse statuses (quarantined or
revoked) proven under the new profile for earlier entries are persisted as
sticky suspensions plus context-tagged observation records while those
entries stay bound to their previous context, the report names them, and the
retry, in which they may be Registered again, carries the restriction across
so only an explicit `enable` clears it. Nothing is ever fabricated for an
entry that was not proven.

A failed save is reported by its phase: before the new lockfile was published
the previous one is intact and nothing was migrated; after it, the new
contents are visible with unconfirmed durability. The recovery is a locked
re-read plus a re-run with the same profiles: every entry is then already at
the new context, so nothing is rebound and no record is appended, and the
command republishes the unchanged lockfile durably before reporting success
— a second sync failure is still an error. When a later point check aborts a
run, the report says what happened to the safety records of the entries
proven adverse before it: recorded and confirmed durable, written with
unconfirmed durability, or not saved at all (in which case the named entries
are to be treated as suspended until a re-run records them).

The migration is the lockfile's step only. The catalog in the state directory
is bound to the context as well and is rebuilt under the new profile with
`intend update`, before or after the migration. The deployment-era profile
remains published as a release asset next to every new one, so `--from` is
always available.

## Profile (spec §3 — the trust configuration; never from a provider)

```toml
chain_id = 100
genesis_hash = "0x…"             # pinned chain identity — checked against EVERY RPC
registry = "0x…"                 # pinned at production deployment
registry_code_hash = "0x5a6cf793…" # keccak256 of the deployed runtime code
anchor_rpcs = ["https://rpc-a…", "https://rpc-b…"]  # ≥2, pairwise-distinct normalized ORIGINS
anchor_operators = ["operator-a", "operator-b"]      # optional; must be pairwise distinct
snapshot_urls = []               # snapshot providers tried in order (bounded fetch + full verify)
registration_meta_evidence = "/ipfs/…" # MANDATORY: the DEPLOYMENT policy refs (version 0); the
clearing_meta_evidence = "/ipfs/…"     # on-chain half is metaEvidenceUpdates PROVEN ACCEPTED
arbitrator = "0x9C1d…9002"             # MANDATORY: the court; proven at every anchor (slot 0)
arbitrator_extra_data = "0x…"           # MANDATORY: court id + jurors; proven byte for byte (slot 1)
governor = "0x…"                       # MANDATORY: the timelock in front of the Safe; proven (slot 3)

[[policy_updates]]                     # OPTIONAL: policy versions accepted beyond the deployment one
updates = 1                            # the metaEvidenceUpdates value that announced this version
registration_meta_evidence = "/ipfs/…" # the references that announcement declared
clearing_meta_evidence = "/ipfs/…"
provider_rpc = "https://rpc-c…"  # untrusted snapshot/proof source (verified locally)
gateways = ["https://ipfs-gateway…"] # untrusted CAR sources (every block hash-checked)
```

The two MetaEvidence references must be canonical `/ipfs/<CIDv1-base32>[/path]`
values and mutually distinct — a strict SUPERSET of the rules
`DeployRegistry.s.sol` enforces at deployment (the client additionally
requires the CID to strictly decode and forbids empty path segments;
strengthening the deployment script to match is pre-production work). The profile is a locally authenticated deployment manifest:
form and distinctness are checked here, the on-chain half is the proof that
`metaEvidenceUpdates` is 0 or one of the listed `policy_updates` values, and
that these are the references the deployment and update events actually
declared is the release manifest's assertion. The policy is mutable behind a
seven-day timelock (RFC 0001 §9): a change the profile does not list fails
closed until a new signed profile names its version. Every request keeps the
version it was submitted under; a removal request opened after a change is
judged under the version then in force, with entries protected only from
classification rules introduced after their admitting request's version
(listing policy 2.3, Amendments).

Every RPC — anchor sources and proof providers alike — is authenticated against
the pinned `chain_id` + `genesis_hash` before use (the genesis check is an
identity comparison; field-consuming anchor headers are re-hashed from their
fields); quorum requires (hash, stateRoot, timestamp) agreement; header time is
bounded in both directions (stale AND future-skew fail closed); all RPC
operations carry deadlines and a response-body byte cap. Every HTTP body —
snapshots, CARs, and JSON-RPC responses alike — is consumed through a
BLOCKING take-bounded reader: at most cap + 1 bytes ever reach the
application's buffer (§4.1; counting-reader tests prove the reader-layer
consumption bound). Below that reader, the locked HTTP stack (Cargo.lock:
reqwest 0.12.28, hyper 1.11.0, h2 0.4.18) buffers response bytes ahead of
consumption within PER-CONNECTION / PER-STREAM configuration bounds —
HTTP/1's read buffer (≤ 417,792 bytes) or the explicitly configured HTTP/2
windows (1 MiB stream, 2 MiB connection, 16 KiB frames; multiple frames may
queue under them) — plus one adapter `Bytes` chunk and platform-managed
TLS/socket buffers. Each component is response-size-independent and never
appended to the application buffer; aggregate memory scales with connections
and concurrency (the CLI's command paths are sequential; no process-wide cap
is enforced by the transport type). State
read-modify-write (catalog, high-water, lockfile) is serialized with
interprocess `flock`s, and the lockfile is proven writable BEFORE content
publishes. If the FINALIZING save fails after publication, nothing is rolled
back — published content is never deleted against a surviving record; the
full-manifest pending journal stands and `audit`/`enable` recover it.

`test_headerless_state_root = true` exists for fork/dev chains ONLY (anvil fork
headers carry a zero stateRoot; the root is then derived from a provider probe
proof and every output labels the degradation). Production profiles must not
set it, and the CLI refuses it against chains that serve real state roots.

## Tests

```bash
cargo test --locked    # 83 offline tests: schema/profile units (incl. strict
                       # MetaEvidence CID decode + deployment-context-id
                       # variation) + counting-reader consumption proofs for
                       # the §4.1 take-bound; CAR tamper suite
                       # (traversal, root substitution, rehash-under-valid-key,
                       # shared blocks, chunked-File lies, duplicate-metadata
                       # rejection, coalescing guard, canonical varints,
                       # NO-REPLACE rename, filesystem-alias probes, lock
                       # exclusivity); the install-transaction suite
                       # (repeat-install preservation, full-manifest journal,
                       # CHILD-PROCESS crash kills at four exact boundaries with
                       # per-boundary recovery — ordinary retry, fail-closed
                       # manual, production enable_transition — retargeted-parent
                       # fail-closed identity, fd-relative cleanup,
                       # deployment-context refusals); catalog/high-water context
                       # binding; lockfile identity (symlink/hard-link/reserved
                       # names); HTTP/RPC boundary caps; kubo interop vectors incl. the
                       # metadata-discard assertions; spec §6 adversarial suite
                       # over REAL in-memory tries (HashBuilder) incl. the EOA
                       # wrong-account fixture and a true exclusion proof — no
                       # network, no anvil.
```

`tools/gen-kubo-vectors.sh` deterministically regenerates BOTH `fixtures/kubo/`
vectors — default and the metadata-bearing (mode+mtime, exec-bit) tree — with
TZ, every file AND directory mode, mtimes, and the kubo version all pinned; it
asserts the default root is unchanged and the metadata root matches its pin
(reproduced identically under three ambient TZ/umask environments; needs kubo;
offline).

## Local end-to-end smoke: `tools/smoke.sh` (reproducible)

`tools/smoke.sh` runs the full lifecycle against a TWO-NODE quorum: anvil A
forks Gnosis at a pinned block; anvil B forks A (identical history, genuinely
distinct normalized origin; B is re-forked after each on-chain mutation so the
quorum's min-finalized advances). It seeds via the Gate 2 spike, registers an
item whose Tree CID is the kubo fixture root, then: `update` (26 items,
complete) → `install` from the fixture CAR (fresh Registered point check,
SKILL.md binding, 9 files) → `audit` current → on-chain removal → `audit`
`revoked` exit 1 → re-registration → `audit` `reenable-required` (sticky held)
exit 1 → `enable` → `audit` current exit 0.

What the smoke does NOT prove: operator independence (both nodes are local —
`anchor_operators` labels them honestly) and header-authenticated state roots
(anvil fork headers carry ZERO state roots; the profile sets the
loudly-labeled `test_headerless_state_root` workaround, which the CLI refuses
on chains serving real roots). Anvil quirks the script handles: fresh clock
warp, `--slots-in-an-epoch 1`, no `--prune-history`, the two-block finality
margin.

Per listing-policy v2.1 (owner decision): skill trees contain directories and
regular files only — symlinks are prohibited and executable bits are
non-semantic, so the installer's behavior (reject symlinks, don't preserve
mode) is the policy, not a gap.

## Deliberately not here (tracked in spec §11)

Strict light-client anchoring (Gate 1 integration); binary wire framing and
final limit values; delta updates (needs upstream `historyHead`); production
profile values (owner-gated at deployment).
