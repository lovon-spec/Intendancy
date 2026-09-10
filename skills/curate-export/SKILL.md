---
name: curate-export
description: "Use when an agent needs the contents of a Kleros Light Curate registry, such as the Kleros Scout lists on Gnosis (Address Tags, Tokens, Contract Domain Names, Address Tags Query), as a file it can verify rather than trust: to check whether an address is already tagged, to review entries pending challenge, to integrate a token list, or to publish a verified snapshot. DOWNLOADS AND RUNS AN EXTERNAL BINARY: curate-export from the github.com/lovon-spec/Intendancy release pinned in scripts/install-curate-export.sh by executable digest; the installer verifies the release signature and the digests and aborts on mismatch before anything runs. The tool proves every item's status in contract storage at a header-quorum anchor, binds each item file by hash, and writes deterministic snapshots, offline lookups, a Uniswap-schema token list, and diffs. Completeness rests on two independent log sources agreeing."
license: MIT
compatibility: "Requires network access and a POSIX shell with curl, tar, gpg 2.x, and sha256sum or shasum. Installs the curate-export binary under $INTEND_HOME (default ~/.intend). Linux and macOS, x86_64 and aarch64."
---

# Verified snapshots of Kleros Curate lists

A Kleros Light Curate list is a challengeable registry: anyone may submit an entry with a deposit, anyone may challenge it, and jurors decide against the list's policy. The Kleros Scout registries on Gnosis are four such lists: Address Tags, Tokens, Contract Domain Names and Address Tags Query. Consuming them used to mean trusting an indexer, an API, or a published export. `curate-export` produces the list from the chain itself: it proves every candidate's status in the contract's storage at a finalized header agreed by independent RPCs, fetches each item file as hash-verified IPFS blocks, and writes a snapshot that is byte-identical for anyone who reruns it at the same anchor.

What the tool cannot prove: that no item was hidden. A Light list keeps no item list in storage, so candidates come from `NewItem` logs, taken from two independently operated RPCs that must agree. The provenance file states this, and it names the anchor mode, a header quorum, which is an alpha trust assumption rather than a light client.

## 1. Install the tool, verified

From the root of this skill tree:

```sh
sh scripts/install-curate-export.sh
```

The installer downloads one fixed release, requires the release signing key to match the pinned fingerprint and every signature to be made under it, checks the archive and the extracted executable against the digests pinned in the script, and installs `$INTEND_HOME/bin/curate-export` (default `~/.intend`). On any mismatch it installs nothing; stop, and do not obtain the binary another way.

Until a release ships `curate-export`, the installer's version and digest pins are `PENDING_RELEASE` placeholders and the script refuses to run; this skill is not submitted to the registry before that release pins them.

```sh
CE="$HOME/.intend/bin/curate-export"
```

## 2. Snapshot a registry

```sh
"$CE" export --registry address-tags --out items.json --provenance provenance.json --csv items.csv
```

Presets: `tokens`, `address-tags`, `atq`, `cdn`, each one deployment on Gnosis. Any other Light list needs `--list <address>` and `--items-slot`, plus `--chain-id` and `--genesis` off Gnosis; a preset name is not combined with another address or chain. `--anchor-block <n>` reruns at an earlier export's anchor and reproduces its `items.json` byte for byte. By default the snapshot holds Registered and ClearingRequested items; add `--include-pending` for the entries under review, which is what a challenger wants to inspect, or `--all` for everything.

Read the JSON line the command prints: `ok` must be `true`; `anchorMode` names the anchor; `byStatus` is the proven status histogram; `fetchFailures` counts item files no gateway could serve. Report the anchor block and mode whenever you hand the snapshot to anyone; `rpc`, `ipfs` and `stageSeconds` say what the run measured. Each item in `items.json` carries its id, status, IPFS path, and the item file's `columns` and `values` verbatim; `items.csv` flattens the values by column label for spreadsheets.

## 3. Query offline

```sh
"$CE" lookup --items items.json --address 0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48
"$CE" lookup --items items.json --value kleros.io --status 1
"$CE" lookup --items items.json --status 2 --json
```

`--address` matches any address or `eip155:<chain>:<address>` value, case-insensitively; `--value` is a case-insensitive substring over every value; `--status` is the status number (0 absent, 1 registered, 2 registrationRequested, 3 clearingRequested). Everything is local; no network.

## 4. The Tokens list as a token list

```sh
"$CE" tokenlist --items tokens-items.json --compare https://t2crtokens.eth.limo/ --out tokens.json --diff-out diff.json --skipped-out skipped.json
```

Renders a Tokens-list snapshot into the Uniswap token-lists schema (chain id and EIP-55 address from the rich address, validated decimals, `ipfs://` logos), deterministically, named `Kleros Tokens Verified` unless `--name` gives another name the schema accepts (letters, digits, underscore, space; up to 30 characters); the output is validated against the schema before it is written. `--previous` applies token-lists versioning; `--compare` writes a diff: tokens only in ours, only in theirs, metadata differences. A token in ours and absent in theirs is one the other export dropped; the provenance proves ours was Registered at the anchor. An item whose Address has no `eip155:` namespace (the list also holds Solana tokens) is skipped with a recorded reason (`--skipped-out` writes the list), the rule Kleros's own exporter applied; no chain is guessed for an address without one.

## 5. Publish

Pin the snapshot or token list together with its provenance (any pinning service; `ipfs add --cid-version 1`) and give consumers the CID with the anchor block. Anyone who distrusts you reruns the command and compares.

## Rules

- Never edit a snapshot by hand; regenerate it.
- Say which anchor mode the export was made under, and that completeness rests on log agreement.
- Do not bypass a failed installer check by downloading the binary elsewhere.
