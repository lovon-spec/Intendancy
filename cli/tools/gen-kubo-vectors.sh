#!/bin/sh
# Regenerates fixtures/kubo/ — BOTH kubo interop vectors for the UnixFS-basic
# profile (spec §11 item 2), deterministically ACROSS ENVIRONMENTS. Requires
# kubo (`brew install ipfs`) at the PINNED version below; runs fully offline
# against a throwaway repo.
#
# The committed tree under fixtures/kubo/tree/ is the byte source of truth.
# Generation works on a TEMP COPY with FULLY PINNED metadata (round-4: local
# time and unnormalized directory modes made earlier output
# environment-dependent):
#   - TZ=UTC is exported, so `touch -t` resolves identically everywhere;
#   - every DIRECTORY is chmod 0755 (--preserve-mode records directory modes);
#   - every file is chmod 0644, except references/large.bin at 0755 (the
#     exec-bit coverage);
#   - mtime 2026-01-01T00:00:00Z on every node.
# Vector 1 (tree.car / root-cid.txt): default `ipfs add -rQ --cid-version 1`
#   — metadata does NOT affect this vector; the script ASSERTS the root still
#   equals the committed root-cid.txt (smoke + the producer-side test pin it).
# Vector 2 (tree-mode.car / mode-root-cid.txt): the same tree with
#   `--preserve-mode --preserve-mtime` — a metadata-bearing tree (mode AND
#   mtime fields, one executable file) whose leaves kubo emits as inline-data
#   File nodes (raw leaves off). The expected root is PINNED below and
#   asserted; regenerating after an intentional tree change requires
#   ALLOW_ROOT_CHANGE=1 (and updating the pins).
set -eu
export TZ=UTC
KUBO_PIN=0.43.0
EXPECTED_MODE_ROOT_FILE_UPDATE_NEEDS=ALLOW_ROOT_CHANGE
here="$(cd "$(dirname "$0")/.." && pwd)"
scratch="$(mktemp -d)"
export IPFS_PATH="$scratch/repo"
trap 'rm -rf "$scratch"' EXIT

kubo_version="$(ipfs version --number)"
if [ "$kubo_version" != "$KUBO_PIN" ]; then
  echo "FAIL: kubo $kubo_version != pinned $KUBO_PIN — vectors are pinned to one kubo version;" >&2
  echo "      update the pin AND both vectors together (deliberately)." >&2
  exit 1
fi

cp -R "$here/fixtures/kubo/tree" "$scratch/tree"
find "$scratch/tree" -type d -exec chmod 0755 {} +
find "$scratch/tree" -type f -exec chmod 0644 {} +
chmod 0755 "$scratch/tree/references/large.bin"
find "$scratch/tree" -exec touch -t 202601010000.00 {} +

ipfs init --profile test >/dev/null 2>&1

root="$(ipfs add -rQ --cid-version 1 --offline "$scratch/tree")"
committed="$(cat "$here/fixtures/kubo/root-cid.txt")"
if [ "$root" != "$committed" ]; then
  echo "FAIL: default-vector root $root != committed root-cid.txt $committed" >&2
  echo "      (metadata must not affect the default vector; the tree bytes changed?)" >&2
  exit 1
fi
ipfs dag export "$root" > "$here/fixtures/kubo/tree.car" 2>/dev/null

mode_root="$(ipfs add -rQ --cid-version 1 --offline --preserve-mode --preserve-mtime "$scratch/tree")"
if [ "$mode_root" = "$root" ]; then
  echo "FAIL: metadata vector root equals the default root — metadata was not recorded" >&2
  exit 1
fi
# The pin is REQUIRED (a missing pin file must not silently bypass the
# check), and the override must be EXACTLY "1" (round-5: any non-empty value,
# e.g. ALLOW_ROOT_CHANGE=0, previously bypassed it).
if [ "${ALLOW_ROOT_CHANGE:-}" != "1" ]; then
  if [ ! -f "$here/fixtures/kubo/mode-root-cid.txt" ]; then
    echo "FAIL: fixtures/kubo/mode-root-cid.txt (the metadata-root pin) is missing" >&2
    echo "      — regenerating without a pin requires $EXPECTED_MODE_ROOT_FILE_UPDATE_NEEDS=1" >&2
    exit 1
  fi
  pinned_mode_root="$(cat "$here/fixtures/kubo/mode-root-cid.txt")"
  if [ "$mode_root" != "$pinned_mode_root" ]; then
    echo "FAIL: metadata-vector root $mode_root != pinned $pinned_mode_root" >&2
    echo "      (non-reproducible environment, or an intentional change without" >&2
    echo "      $EXPECTED_MODE_ROOT_FILE_UPDATE_NEEDS=1)" >&2
    exit 1
  fi
fi
ipfs dag export "$mode_root" > "$here/fixtures/kubo/tree-mode.car" 2>/dev/null
printf '%s\n' "$mode_root" > "$here/fixtures/kubo/mode-root-cid.txt"

{
  printf 'generator: tools/gen-kubo-vectors.sh (deterministic across environments; both vectors)\n'
  printf 'kubo: %s (pinned)\n' "$kubo_version"
  printf 'deterministic metadata: TZ=UTC; touch -t 202601010000.00 on every node; chmod 0755 all dirs; chmod 0644 all files; chmod 0755 references/large.bin (exec-bit coverage)\n'
  printf 'default vector: ipfs add -rQ --cid-version 1 --offline <tree>; ipfs dag export <root>\n'
  printf 'default root: %s\n' "$root"
  printf 'car-sha256: %s\n' "$(shasum -a 256 "$here/fixtures/kubo/tree.car" | cut -d' ' -f1)"
  printf 'metadata vector (mode+mtime): ipfs add -rQ --cid-version 1 --offline --preserve-mode --preserve-mtime <tree>; ipfs dag export <root>\n'
  printf 'metadata root: %s\n' "$mode_root"
  printf 'mode-car-sha256: %s\n' "$(shasum -a 256 "$here/fixtures/kubo/tree-mode.car" | cut -d' ' -f1)"
} > "$here/fixtures/kubo/PROVENANCE.txt"
echo "default root: $root"
echo "metadata root: $mode_root"
