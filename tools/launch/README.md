# Launch tooling

Scripts for the launch inputs that need an account or a key the maintainer holds (the governor Safe, the timelock in front of it, the pinning account) and for the assets the registry's MetaEvidence points at. Each script does one step and fails closed; none of them reads or prints a key. Values and decisions live in `docs/launch-readiness.md`.

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

## The governor timelock (launch-readiness row 14)

The registry's governor is a `TimelockController` (OpenZeppelin v5.7.0) whose only proposer and canceller is the Safe; execution is open, the timelock administers itself, and its delay is seven days. It was deployed on 2026-09-07 by the Safe itself, not by a funded key.

**The proposer route.** A Safe cannot run `CREATE`, so it calls the standard CREATE2 deployer contract `0x4e59b44847b379578588920cA78FbF26c0B4956C` with `salt ++ initcode` as raw calldata; the deployer runs `CREATE2`, and the address follows from the deployer, the salt and the initcode alone, so it is known before anyone signs. The proposal itself comes from a key the Safe's owner registered as a *proposer* in the Safe app (Settings, Setup, Proposers: a transaction-service delegate). The proposer signs the Safe transaction hash and posts it to the Safe Transaction Service; the owner sees it in the queue, compares the hash, confirms and executes. Queued transactions execute in nonce order. The proposer key pays no gas and holds no role on the Safe, the timelock or the registry.

```bash
tools/launch/timelock-create2.sh <safe> <out-dir>          # initcode + calldata; prints the CREATE2 address and both hashes
tools/launch/propose-safe-tx.sh <safe> 0x4e59b44847b379578588920cA78FbF26c0B4956C <out-dir>/calldata.hex \
    --origin "governor timelock" --key-file ~/.intendancy/keys/proposer.key   # omit --key-file for a dry run: prints the safeTxHash only
tools/launch/verify-timelock.sh <timelock> --proposer <safe> --deployer 0x4e59b44847b379578588920cA78FbF26c0B4956C --rpc <url>
```

`timelock-create2.sh` takes the creation code from this repository's own build (`forge inspect --json TimelockController bytecode`; the bare contract name, the path form fails for a contract under `lib/`) and appends the constructor arguments `DeployTimelock.s.sol` uses: delay 604800, proposers `[the Safe]`, executors `[address(0)]`, admin `address(0)`. `propose-safe-tx.sh` asks the Safe itself for the transaction hash (`getTransactionHash`, operation Call, no refund fields), signs it with `cast wallet sign --no-hash` and posts to the Safe{Core} API for Gnosis (`https://api.safe.global/tx-service/gno`; the older per-chain host only redirects there, and no API key was needed); it then reads the queued entry back and prints its nonce, target, data keccak and proposer. `verify-timelock.sh` proves the result against a live RPC: runtime code equal to the repository's build, delay, the Safe's proposer and canceller roles, open execution, self-administration, and no role for the deployer or the proposer beyond those.

**Mainnet record, 2026-09-07.**

| Item | Value |
|---|---|
| Timelock | `0xc8Ba4c0AD3554EDB0a9A4C8D73Bf87410A313ADa` |
| Safe transaction | nonce 0, safeTxHash `0x629f2cb9c1d20e76abdc18f485fdac7240c948989cd5fd8de163e5609166e959` |
| Mined | tx `0x68fd0f8a84bdc89930b64ce55c5cc1e318aa52aa8efaad1cd0e8c20a21a81b5b`, block 48119043 |
| Calldata | 7041 bytes, keccak `0x4184461460a3c4b6eea61fbb9cf3e56d0124866249a7d30ac1bd823cd4221208` (zero salt) |
| Initcode hash | `0x6623c3f24e5a14adaf95cc88f07d1f4b14effe0057b150add8a02c84397012a0` |
| Runtime code hash | `0x45e88f95d644dd4efbe0d33617197effb21a61f4baa1cfa48eb3bd1af030f261` |
| `verify-timelock.sh` on mainnet | every check passes; delay 604800 s, proposer and canceller the Safe `0x55705C596E087866C2B8e9339D7016b35a32619F` |
| Fork rehearsal | the same Safe transaction executed on a fork at block 48118466 with the owner impersonated: 1,493,281 gas, same address, same checks |

**Test list, same day.** For a last look at the registry in the official Curate interface, a clearly labelled TEST list was queued as Safe nonce 1 (safeTxHash `0xbff020ac8ece243fa94b27b1e2f3f539c9a651e4c865d59d45d3c04ab9f84b26`, mined in the same tx) and lives at `0x2A25ec70329918d13Ec23477716a159D063735D0`: the production configuration except a 3600 s challenge period and the proposer key as governor, with MetaEvidence titled "TEST: Agent Skills Registry (test deployment, not the registry)". It is not the registry, never goes into a profile, and is disabled after the review (deposits raised out of reach, then the governor handed to a dead address). Details in `docs/launch-readiness.md`, section 5.

## Pinned assets (launch-readiness row 8)

The MetaEvidence files reference three IPFS paths: the evidence display (`/ipfs/<cid>/index.html`), the registry logo (`/ipfs/<cid>/agent-skills-registry-logo.svg`) and the policy PDF (`/ipfs/<cid>/listing-policy.pdf`). The CIDs are computed here, with kubo's `--cid-version 1` defaults in a throwaway repository, and the pinning service must report the same CID, which is why the upload is a CAR import rather than a file upload that the service chunks its own way.

```bash
tools/launch/build-display.sh display.car                 # builds frontend/dist-evidence for production, prints the CID
tools/launch/car-of.sh --wrap meta-evidence/agent-skills-registry-logo.svg logo.car
tools/launch/render-policy.sh listing-policy.pdf          # the policy has no placeholders left; keep the file
tools/launch/car-of.sh --wrap listing-policy.pdf policy.car
tools/launch/verify-car.sh display.car <cid>              # what a consumer checks: CID forms, hashes, completeness
tools/launch/pin-filebase.sh display.car <bucket>         # S3 CAR import; compares the service's CID with the CAR root
tools/launch/check-gateways.sh <cid>                      # fetches the DAG as a CAR from public trustless gateways and verifies it
```

`build-display.sh` is reproducible: two builds from the same commit produce byte-identical CARs. `render-policy.sh` is not, because Chrome stamps a creation date, so the PDF is rendered once on deployment day and that exact file is pinned and kept.

The service is Filebase (bucket `agent-skills-registry`): its S3 API imports a CAR when the object carries the metadata `import=car` and, a few seconds after the upload returns, reports the root CID in the object's `cid` metadata and `pinned` in `pinning-status`; `pin-filebase.sh` polls for the metadata (up to two minutes) and compares the CID with the CAR root. Credentials come from the maintainer's own AWS CLI configuration. The script has been run against the live account for every asset below, and public gateways served each CID within minutes of the import. Storacha is the alternative (`w3 up --car display.car` with its CLI logged in), with the same `check-gateways.sh` proof afterwards. Whatever service is used, `check-gateways.sh` is the acceptance test: the court and the CLI fetch from public gateways, not from the service's own gateway.

Pinned as of 2026-09-07 and verified from `dweb.link`, `ipfs.io`, `trustless-gateway.link` and the Kleros CDN. The display CID changes with any change to the display's source or dependencies, so it is recomputed on deployment day (the build is reproducible: PR #16 left it unchanged).

| Asset | CID | Size | Status |
|---|---|---|---|
| Evidence display bundle | `bafybeihbs35rkglz4kfaivcomd54ntnd4ec24cnlcspxhbol32jxg3eql4` | 11 blocks, 843,645 bytes | pinned; the production bundle |
| Registry logo, wrapped (wordmark, `tools/logo/build-logo.py`) | `bafybeidin5z2vh6i7at2xufs3574sxzyhc7ibzslmzwj7ubthuzazwjvge` | 2 blocks, 10654 bytes | pinned 2026-09-07; the production logo, served by the Kleros CDN |
| Former tile logo, wrapped | `bafybeicjara72feyuvhuj7ej76ra5kkblpixn7rskabhcsxy5ezyadhxu4` | 2 blocks, 658 bytes | pinned; used only by the TEST list's meta-evidence |
| Policy PDF, TEST copy | `bafybeifcum2rnx6ro2mqqqxqzijbm2juxxm5faoe54bnjxbxsf4mpll7ba` | 2 blocks, 174,225 bytes | pinned; the PR #16 policy with the timelock address filled and a TEST banner on top, for the test list only |
| MetaEvidence, TEST copies | registration `bafybeih3jgo6rruhs73u6xhxlq455qmvhmpb3v6lysmptnwrkbtofn2w4m`, clearing `bafybeie3xoh2rbyhbszyqa7djg6dnx4cyhhpvogzgowo6spk24jxpagd64` | | pinned; TEST-labelled, for the test list only |
| Seed trees, five | see `docs/launch-readiness.md` row 11 | | pinned |
| Policy PDF and MetaEvidence, production | rendered and pinned on deployment day from the merged policy | | pending |

`verify-car.sh` was checked against a CAR with one flipped byte (fails on the block hash), a wrong expected root (fails), a non-CAR file (fails) and the kubo interop fixture served by a local kubo gateway through `check-gateways.sh` (passes; the same CID from public gateways fails with HTTP 520 because it was never pinned there, which is the failure mode the check exists to catch).

## Deployment-day order

1. Done: the Safe (`verify-safe.sh`) and the timelock (`timelock-create2.sh`, `propose-safe-tx.sh`, `verify-timelock.sh`); both addresses are in the policy.
2. Me: render the production PDF from the merged policy; build the display and confirm its CID; write the three CIDs into both MetaEvidence files; produce the CARs; pin them with `pin-filebase.sh`; `check-gateways.sh` on each CID. Only then are the MetaEvidence files final and the factory call's `REG_META_EVIDENCE` and `CLEAR_META_EVIDENCE` URIs known.
3. Me: queue the factory's `deploy` call in the Safe with `propose-safe-tx.sh`, with the values of `docs/launch-readiness.md` section 4; owner: compare the hash, sign, execute.
4. Me: take the registry address from the mined `NewGTCR` log, verify the configuration on chain, then fill the profile.
