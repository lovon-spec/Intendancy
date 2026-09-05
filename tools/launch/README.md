# Launch tooling

Scripts for the two launch inputs that need an account or a key the maintainer holds (the governor Safe and the pinning account) and for the assets the registry's MetaEvidence points at. Each script does one step and fails closed; none of them reads or prints a key. Values and decisions live in `docs/launch-readiness.md`.

Requirements: Foundry `cast`, kubo `ipfs`, Node with the frontend's dependencies installed (`cd frontend && npm ci`), Python 3 with the `markdown` package, headless Chrome for the PDF, and the AWS CLI for the Filebase upload.

## The governor Safe (launch-readiness row 7)

The registry's governor is a one-of-one Safe v1.4.1 on Gnosis Chain, created through the canonical `SafeProxyFactory` with the canonical `SafeL2` singleton and `CompatibilityFallbackHandler`. Two ways to create it:

- **Safe web app.** At app.safe.global choose Gnosis Chain, one owner (the maintainer's EOA), threshold 1. The app deploys exactly the canonical contracts.
- **From the terminal.** `safe-create.sh <owner-eoa>` builds the `setup` initializer, asks the factory for the CREATE2 address the Safe will get, and prints the exact `cast send` line. The key is never an argument: `cast` prompts for it when the line is run, or when the script is run with `--broadcast`.

Either way, before the address goes into `GOVERNOR`, the policy or the profile:

```bash
tools/launch/verify-safe.sh <safe-address> --owner <owner-eoa>
```

It proves, against a live RPC, that the address holds the SafeProxy v1.4.1 runtime, that its singleton and fallback handler are the canonical v1.4.1 contracts with the code hashes recorded in `safe-global/safe-deployments`, that `VERSION()` is `1.4.1`, that there is exactly one owner (the expected one when `--owner` is given), threshold 1, no guard and no module. Any deviation is a failure and a non-zero exit.

Evidence, 2026-09-05, on an Anvil fork of Gnosis at block 48083468 with Anvil's public test account standing in for the owner:

| Step | Result |
|---|---|
| Canonical contracts on Gnosis mainnet | `SafeL2`, `SafeProxyFactory`, `CompatibilityFallbackHandler` and `Safe` v1.4.1 code hashes match the safe-deployments records |
| `safe-create.sh` prediction vs. mined `ProxyCreation` log | identical: `0xF0c3E2d64Ae3EFa6471309f44b10Fce416ccAfd8` (gas 259,931) |
| `verify-safe.sh` on that Safe | every check passes; exit 0 |
| `verify-safe.sh` on an EOA, on the Kleros arbitrator proxy, and with a wrong `--owner` | 6, 6 and 1 failed checks; exit 1 |

The SafeProxy runtime hash the verifier pins, `0xd7d408eb…31fb4c`, is the keccak of the 171-byte runtime segment that the factory's `proxyCreationCode()` returns (offset 281 of its 486 bytes) and of the code found at the created proxy.

## Pinned assets (launch-readiness row 8)

The MetaEvidence files reference three IPFS paths: the evidence display (`/ipfs/<cid>/index.html`), the registry logo (`/ipfs/<cid>/agent-skills-registry-logo.svg`) and the policy PDF (`/ipfs/<cid>/listing-policy.pdf`). The CIDs are computed here, with kubo's `--cid-version 1` defaults in a throwaway repository, and the pinning service must report the same CID, which is why the upload is a CAR import rather than a file upload that the service chunks its own way.

```bash
tools/launch/build-display.sh display.car                 # builds frontend/dist-evidence for production, prints the CID
tools/launch/car-of.sh --wrap meta-evidence/agent-skills-registry-logo.svg logo.car
tools/launch/render-policy.sh listing-policy.pdf          # after [GOVERNOR_ADDRESS] is filled; keep the file
tools/launch/car-of.sh --wrap listing-policy.pdf policy.car
tools/launch/verify-car.sh display.car <cid>              # what a consumer checks: CID forms, hashes, completeness
tools/launch/pin-filebase.sh display.car <bucket>         # S3 CAR import; compares the service's CID with the CAR root
tools/launch/check-gateways.sh <cid>                      # fetches the DAG as a CAR from public trustless gateways and verifies it
```

`build-display.sh` is reproducible: two builds from the same commit produce byte-identical CARs. `render-policy.sh` is not, because Chrome stamps a creation date, so the PDF is rendered once on deployment day and that exact file is pinned and kept.

The proposed service is Filebase: its S3 API imports a CAR when the object carries the metadata `import=car` and then reports the root CID in the object's `cid` metadata, which `pin-filebase.sh` compares with the CAR root. Credentials come from the maintainer's own AWS CLI configuration; the script has not yet been run against a live account. Storacha is the alternative (`w3 up --car display.car` with its CLI logged in), with the same `check-gateways.sh` proof afterwards. Whatever service is used, `check-gateways.sh` is the acceptance test: the court and the CLI fetch from public gateways, not from the service's own gateway.

Provisional CIDs as of `main` 3e53cac9 (they change with any change to the display's source or dependencies, so recompute on deployment day):

| Asset | CID | Size |
|---|---|---|
| Evidence display bundle | `bafybeihbs35rkglz4kfaivcomd54ntnd4ec24cnlcspxhbol32jxg3eql4` | 11 blocks, 843,645 bytes |
| Registry logo, wrapped | `bafybeicjara72feyuvhuj7ej76ra5kkblpixn7rskabhcsxy5ezyadhxu4` | 2 blocks, 658 bytes |
| Policy PDF | known only after the final render | |

`verify-car.sh` was checked against a CAR with one flipped byte (fails on the block hash), a wrong expected root (fails), a non-CAR file (fails) and the kubo interop fixture served by a local kubo gateway through `check-gateways.sh` (passes; the same CID from public gateways fails with HTTP 520 because it was never pinned there, which is the failure mode the check exists to catch).

## Deployment-day order

1. Owner: create the Safe; run `verify-safe.sh`; send the address.
2. Me: fill `[GOVERNOR_ADDRESS]` in both policy copies; render the PDF; build the display; compute the three CIDs; write them into both MetaEvidence files; produce the three CARs.
3. Owner: pin the three CARs (`pin-filebase.sh` or the Storacha equivalent).
4. Me: `check-gateways.sh` on each CID; only then are the MetaEvidence files final and the deploy script's `REG_META_EVIDENCE` and `CLEAR_META_EVIDENCE` URIs known.
