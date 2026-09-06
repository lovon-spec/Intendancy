# Intendancy launch readiness

One page, one row per remaining input, each with a proposal so the decisions can be taken in one sitting. Status as of 2026-09-04. Nothing here authorizes a deployment or spending; the deployment transaction is the owner's, with the values approved below.

The facts these proposals rest on were checked against Gnosis mainnet on 2026-09-04: the deploy script's four pins (the official `GTCRFactory` code hash, the xKlerosLiquid proxy code hash, its EIP-1967 implementation address and that implementation's code hash) all still match; the factory has deployed 32 registries; court 19 charges 7.2, 21.6 and 36 xDAI for one, three and five jurors; the three Scout lists use court 19 with three jurors.

## 1. Decisions

| # | Input | Proposal | Why | Status |
|---|---|---|---|---|
| 0 | Registry name | The on-chain list is the unbranded **Agent Skills Registry**; Intendancy brands only the CLI distribution, the site and the profile (PR #9). | Neutral infrastructure in Kleros's list of lists is what other runtimes and submitters build on; the name is immutable per registry. | decided 2026-09-04 |
| 1 | Listing policy, last open rule: CIDv0 children | Require CIDv1 for every linked CID in a tree. | It is what `ipfs add --cid-version 1` produces, what the installer already enforces, and what the evidence display flags. A v0 link in a v1 tree only ever comes from a mixed toolchain. | adopted 2026-09-05 (policy 2.2) |
| 2 | Agent Skills revision pin | Pin the head commit of `github.com/agentskills/agentskills` on the day the policy is approved; write it into `[SPEC_COMMIT_HASH]` in both policy copies. | Policy is immutable per registry, so the pin is forever; the newest published revision at approval time is the least surprising choice. | adopted 2026-09-05: `69ef37e9`, the head since 2026-08-09; if the head moves before the policy is pinned on deployment day, re-pin deliberately |
| 3 | Deposits | Submission base deposit 30 xDAI; removal base deposit 30 xDAI; submission and removal challenge base deposits 0. | The Scout precedent. A zero challenger deposit means a challenger risks only the arbitration fee, so there is no bait profit in provoking challenges, and honest challengers of a tiny new list are not asked to post capital. | confirmed 2026-09-06 |
| 4 | Challenge period | 3.5 days, 302400 seconds. | The Scout precedent; long enough for a human watchdog, short enough that listing is not a week-long wait. | confirmed 2026-09-06 |
| 5 | Court and jurors | Court 19, three jurors; arbitration cost 21.6 xDAI today. | The Curation court the Scout lists use; three jurors is the smallest panel Kleros runs there. A listing therefore locks 51.6 xDAI until it executes; a challenge costs 21.6. | confirmed 2026-09-06 |
| 6 | Stake multipliers | Shared 100%, winner 100%, loser 200%, in basis points 10000, 10000, 20000. | Kleros's defaults for appeal crowdfunding; no reason known to deviate. | confirmed 2026-09-06 |
| 7 | Governor | A one-of-one Safe on Gnosis, deployed before the registry; its address goes into `GOVERNOR`. The policy's Governance section (PR #9) discloses it: never the meta-evidence update, arbitrator changes only through an announced RFC 0001 amendment and a signed CLI profile release, parameter changes announced ahead of time, and the wider-governance path described as the registry's community, naming no organization. The Safe address fills `[GOVERNOR_ADDRESS]` in both policy copies before pinning. | Decided on 2026-09-04. A Safe can gain signers later without changing the registry's governor. | done 2026-09-06: the Safe is `0x55705C596E087866C2B8e9339D7016b35a32619F`, verified with `verify-safe.sh` on Gnosis mainnet (one owner, threshold 1, canonical v1.4.1 contracts); it fills `[GOVERNOR_ADDRESS]` |
| 8 | Pinning arrangement | One pinning-service account for the launch assets: the policy PDF, the neutral registry logo, the evidence display bundle, and the seed trees, all added with CIDv1. Retrieval through public trustless gateways in the CLI profile. Submitters pin their own trees, as the policy already requires. | Decided on 2026-09-04. The Kleros gateway serves what it can find but pins nothing we reference. | owner opens the account (Filebase proposed: CAR import over its S3 API, so the pinned CID is exactly the one computed here); `tools/launch/` computes CIDs and CARs, uploads, and proves gateway retrieval |
| 9 | Checkpoint and freshness | Checkpoint embedded in the signed CLI release, cached forward after verification, operator override allowed; maximum catalog age one day. | Decided on 2026-09-04; it is the candidate the spec already names. Every install does a fresh on-chain check regardless of catalog age. | decided; spec text to update |
| 10 | Release signing | SSH-signed git tags for CLI releases, release assets with a SHA-256 manifest signed by the maintainer's minisign key, the profile file shipped as a release asset. | Consumers verify a profile before trusting it; a signed manifest is the cheapest way to make that possible without a package registry. | confirmed 2026-09-06 |
| 11 | Seed skills | Five Intendant skills whose trees already exist on this machine: `intendant-coordination`, `intendant-agenda`, `intendant-memory`, `intendant-cli`, `show-then-ask`. Candidates for a second batch: `intendant-log-search`, `intendant-remote-compute`, `visual-collaboration`, `peer-displays`. | Real skills make the first install real. Five listings lock 258 xDAI for the period, returned on execution. Each tree needs a check against the policy before submission: spec-defined frontmatter fields only, no symlinks, under 2 MiB, CIDv1. | confirmed 2026-09-06 |
| 12 | Frontend hosting | The site on a static host with HTTPS and a custom domain; the evidence display bundle on IPFS regardless, because the court loads it from there. | The site is static and reads the chain and the public subgraph; nothing needs a server. | confirmed 2026-09-06 |
| 13 | Watchdog at launch | We are the watchdog: a monitor on `RequestSubmitted` events plus manual review, and a challenger wallet funded with at least two arbitration fees. | A new list has no deposit-hunters yet; the policy's availability rule and the challenge period only protect the list if someone is watching. | confirmed 2026-09-06; owner funds the wallet |
| 14 | Governor timelock, and with it a mutable policy | The registry's governor is a TimelockController, never the Safe directly: the Safe proposes and may cancel, anyone may execute a matured operation, the timelock administers itself, and every governor action, deposits, challenge period, stake multipliers, the arbitrator and its extra data, the MetaEvidence and the governor itself, takes effect no earlier than seven days after it is queued. The timelock's address is `GOVERNOR`; the Safe is `GOVERNOR_PROPOSER` and fills `[GOVERNOR_ADDRESS]` in the policy, the timelock fills `[TIMELOCK_ADDRESS]`. The policy therefore becomes mutable with notice: Classic GTCR stores the meta-evidence id on every request, so items and disputes keep the policy they were submitted under, and the CLI accepts the policy versions its signed profile lists and fails closed on any other. | Decided 2026-09-06, reversing the immutable-policy decision of 2026-08-24. A one-shot registry could not take the severity ladder, the credit module or any correction without re-listing every skill, and every Kleros list already runs a mutable policy; the timelock turns "announced ahead of time" into a contract property and is what makes a one-of-one Safe an acceptable proposer. | decided; contracts, tooling and the CLI rule merged (#14, #15); deployment pending: deployer EOA `0xC05e109F24CbE59301272fe8e7a908F67A1af539` (gas only, no role), predicted timelock address `0x9f7Fa2f2C5062056A233C39f4fa95Eb3150b88D2` at its first transaction; dry-run on a fork with the real Safe as proposer passed |

## 2. Engineering before the deployment

| Item | Depends on | Owner |
|---|---|---|
| Fill `[GOVERNOR_ADDRESS]` (`[SPEC_COMMIT_HASH]` filled 2026-09-05), render the policy to PDF, pin it, the neutral logo and the display bundle, write the three CIDs into both MetaEvidence files | rows 7, 8 | me |
| CLI pin of the arbitrator and its extra data: storage-slot probe on a fork, spec sections 3, 5, 6 and 8, profile fields, rogue-governor fixtures and a live switch on a fork that fail closed | done (PR: feat/cli-arbitrator-pin); the slot evidence record stays PROVISIONAL until a reviewer reproduces the probe | me |
| Spec text for the checkpoint policy and the freshness value | row 9 | me |
| Production environment file for the deploy script with the approved values, and a full dry run in production mode on a Gnosis fork, including extracting the registry address from the factory receipt | rows 3 to 7 | me |
| Frontend production environment and build; CLI profile template with everything except the post-deployment values | rows 8, 12 | me |
| RFC 0001 amendment: arbitrator switching as a governance event, with disclosure through the profile | nothing | me |
| Seed-tree preparation: policy check, CIDv1 trees, pinning | rows 8, 11 | me, owner pins |
| Timelock governor: `DeployTimelock`, production-mode checks that `GOVERNOR` is a self-administered timelock with the Safe as proposer, `tools/launch/verify-timelock.sh` | row 14 | done in `feat/timelock-governor`; owner deploys |
| CLI: accept the policy versions the signed profile lists instead of requiring a zero update counter; pin the governor (slot 3) at every anchor; spec sections 3, 5, 6 and 8 | row 14 | me, `feat/cli-policy-versions` |
| The Safe and the pinning account | nothing | owner |
| Launch tooling: Safe creation and verification, CAR/CID computation, CAR verification, gateway retrieval check, pinning upload | done (`tools/launch/`, README has the fork transcript and the provisional display and logo CIDs) | me |

Not on the critical path: the binary framing spike, strict light-client mode, the installer freeze items, and Intendment's adoption. None touches anything on chain.

## 3. Deployment day, in order

1. Owner: deploy the Safe if not done; deploy the timelock with `DeployTimelock` (proposer the Safe, delay 604800 seconds) and verify it with `tools/launch/verify-timelock.sh`; fund the deployer with gas; run the registry deploy script in production mode with the approved environment file, `GOVERNOR` the timelock and `GOVERNOR_PROPOSER` the Safe.
2. Me: take the registry address from the factory's mined `NewGTCR` log, never from the simulation; fetch its code and check the hash against the spec's frozen value; stop if it differs.
3. Me: read the two `MetaEvidence` events from the same receipt and pin their references in the profile; verify the governor, the arbitrator, the extra data, the deposits and the period on chain against the approved values.
4. Me: fill the CLI profile and the frontend environment; build and publish the site; publish the profile in a signed release.
5. Owner: submit the seed skills from the governor's or a dedicated wallet; after the challenge period, execute them.
6. Both: announce, with the registry address, the profile, and the governance note.

## 4. What each deploy variable will hold

| Variable | Value under the proposals above |
|---|---|
| `DEPLOYMENT_MODE` | `production` |
| `GOVERNOR` | the timelock (row 14) |
| `GOVERNOR_PROPOSER` | the Safe; the deploy script checks it holds the timelock's proposer and canceller roles and no admin role |
| `TIMELOCK_PROPOSER`, `TIMELOCK_MIN_DELAY_SECONDS` (for `DeployTimelock`) | the Safe, 604800 |
| `SUBMISSION_BASE_DEPOSIT_WEI`, `REMOVAL_BASE_DEPOSIT_WEI` | 30 xDAI each |
| `SUBMISSION_CHALLENGE_BASE_DEPOSIT_WEI`, `REMOVAL_CHALLENGE_BASE_DEPOSIT_WEI` | 0 |
| `CHALLENGE_PERIOD_SECONDS` | 302400 |
| `MIN_JURORS` | 3 |
| `SHARED_STAKE_MULTIPLIER_BPS`, `WINNER_STAKE_MULTIPLIER_BPS`, `LOSER_STAKE_MULTIPLIER_BPS` | 10000, 10000, 20000 |
| `REG_META_EVIDENCE`, `CLEAR_META_EVIDENCE` | the pinned MetaEvidence files, as IPFS URIs |
| `DEPLOYER_PRIVATE_KEY` | the owner's, never handled by me |
