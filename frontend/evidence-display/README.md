# Evidence display

The page Kleros jurors see inside the court when a request in the Agent Skills Registry is disputed. The court loads it in a frame as `index.html?{"disputeID":…,"arbitrableContractAddress":…,"arbitratorContractAddress":…,"arbitrableChainID":…,"arbitrableJsonRpcUrl":…}` (URL-encoded), which is the convention the registration and clearing MetaEvidence declare under `evidenceDisplayInterfaceRequiredParams`.

What it does, in the browser, with no backend:

1. Maps the dispute to the item through the registry's `arbitratorDisputeIDToItem`, reads and decodes the descriptor, and reads the request's parties.
2. Fetches the complete tree as a CAR from a public gateway and verifies it locally: every block hashed against its CID, DAG-PB and raw only, CIDv1 with 32-byte SHA-256, no symlinks, the installer's UnixFS profile, the policy's 2 MiB bound. The gateway is a transport; the hashes are what is trusted.
3. Checks the frontmatter binding with the same rules as the `intend` CLI, links the Origin, and renders SKILL.md and every text file, behind a notice that the content is evidence and never instructions.

The same page also takes `?registry=0x…&item=0x…[&chain=100][&rpc=…]` for direct use from our own site.

## Build and pin

```bash
npm run build:evidence          # writes dist-evidence/ with relative asset paths
ipfs add -r --cid-version 1 dist-evidence   # or any pinning service; note the directory CID
```

Then set `evidenceDisplayInterfaceURI` in both MetaEvidence files to `/ipfs/<directory CID>/index.html` before the MetaEvidence itself is pinned and passed to the deployment.

Gateways are configurable with `VITE_IPFS_GATEWAYS` (comma-separated) and the RPC with `VITE_RPC_URL`; the court also passes its own RPC, which takes precedence.

## Test

```bash
npm test                        # verifier and frontmatter rules against the CLI's kubo interop vectors
npm run dev:evidence            # local page; append the query form above
```

## Local demo, end to end

Everything runs on this machine: a Gnosis fork with a registry, a local IPFS gateway holding the CLI's kubo fixture tree, and the display.

```bash
# 1. a Gnosis fork on 8545, clock warped to now
anvil --fork-url https://rpc.gnosischain.com --port 8545 --slots-in-an-epoch 1 --silent &
now=$(date +%s); ts=$(cast block latest --rpc-url http://127.0.0.1:8545 --json | python3 -c "import json,sys;print(int(json.load(sys.stdin)['timestamp'],16))")
cast rpc evm_increaseTime $((now - ts)) --rpc-url http://127.0.0.1:8545 >/dev/null; cast rpc anvil_mine 0x2 --rpc-url http://127.0.0.1:8545 >/dev/null

# 2. a registry through the official factory, seeded by the Gate 2 spike
(cd ../spikes/snapshot-bench && cargo run --release --locked -q -- seed --rpc-url http://127.0.0.1:8545 --items 5 --out /tmp/seed.json)
REG=$(python3 -c "import json;print(json.load(open('/tmp/seed.json'))['registry'])")

# 3. the kubo fixture skill as a registration request (RLP recipe in cli/tools/smoke.sh)
#    cast send $REG "addItem(bytes)" "$RLP" --value $COST --private-key $KEY --rpc-url http://127.0.0.1:8545

# 4. a scratch kubo gateway on 8081 serving the fixture tree, offline
export IPFS_PATH=/tmp/intendancy-ipfs; ipfs init; ipfs config Addresses.API /ip4/127.0.0.1/tcp/5011
ipfs config Addresses.Gateway /ip4/127.0.0.1/tcp/8081; ipfs config --json Addresses.Swarm '[]'
ipfs dag import ../cli/fixtures/kubo/tree.car; ipfs daemon --offline &

# 5. the display, local gateway first
VITE_IPFS_GATEWAYS="http://127.0.0.1:8081,https://dweb.link" npm run dev:evidence -- --host 127.0.0.1 --port 5174 --strictPort
# open http://127.0.0.1:5174/?registry=$REG&item=$ID&chain=100&rpc=http://127.0.0.1:8545
```

The fork keeps chain id 100, so `chain=100` with the local `rpc` is correct. Public gateways cannot serve the fixture tree, which is pinned nowhere else; that is why the local gateway comes first.
