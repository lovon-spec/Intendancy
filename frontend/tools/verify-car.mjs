#!/usr/bin/env node
// Verify a CAR file the way the consumer does: every block's CID must be CIDv1
// with a 32-byte SHA-256 multihash and the DAG-PB or raw codec, every block's
// bytes must hash to its CID, and the DAG under the root must be complete.
//
//   node frontend/tools/verify-car.mjs <file.car> [expected-root-cid]
//
// Exit 0 and print "ok <root> blocks=<n> bytes=<m>" on success; exit 1 otherwise.
// Lives under frontend/ so the ESM-only IPLD packages resolve from its
// node_modules; tools/launch/verify-car.sh is the wrapper.
import { createReadStream } from "node:fs";
import { createHash } from "node:crypto";
import { CarReader } from "@ipld/car";
import { CID } from "multiformats/cid";
import * as dagPb from "@ipld/dag-pb";

const DAG_PB = 0x70, RAW = 0x55, SHA2_256 = 0x12;
const [, , file, expected] = process.argv;
if (!file) { console.error("usage: verify-car.mjs <file.car> [expected-root-cid]"); process.exit(2); }

const fail = (msg) => { console.error(`FAIL ${msg}`); process.exit(1); };
const checkCid = (cid, where) => {
  if (cid.version !== 1) fail(`${where}: CIDv${cid.version}, the policy requires CIDv1`);
  if (cid.code !== DAG_PB && cid.code !== RAW) fail(`${where}: codec 0x${cid.code.toString(16)} outside dag-pb/raw`);
  if (cid.multihash.code !== SHA2_256 || cid.multihash.size !== 32) fail(`${where}: multihash is not sha2-256/32`);
};

let reader;
try {
  reader = await CarReader.fromIterable(createReadStream(file));
} catch (caught) {
  fail(`not a readable CAR: ${caught instanceof Error ? caught.message : String(caught)}`);
}
const roots = await reader.getRoots();
if (roots.length !== 1) fail(`CAR has ${roots.length} roots, expected exactly one`);
const root = roots[0];
checkCid(root, "root");
if (expected && root.toString() !== expected) fail(`root ${root} differs from expected ${expected}`);

const blocks = new Map();
for await (const { cid, bytes } of reader.blocks()) {
  checkCid(cid, `block ${cid}`);
  const digest = createHash("sha256").update(bytes).digest();
  if (Buffer.compare(digest, Buffer.from(cid.multihash.digest)) !== 0) fail(`block ${cid}: bytes do not hash to the CID`);
  if (blocks.has(cid.toString())) fail(`block ${cid}: duplicate`);
  blocks.set(cid.toString(), { cid, bytes });
}

// Walk the DAG from the root; every link must be present.
const reached = new Set();
let bytesTotal = 0;
const entries = [];
const stack = [{ cid: root, path: "" }];
while (stack.length) {
  const { cid, path } = stack.pop();
  const key = cid.toString();
  if (reached.has(key)) continue;
  const block = blocks.get(key);
  if (!block) fail(`missing block ${cid} (${path || "root"}): the DAG is incomplete`);
  reached.add(key);
  bytesTotal += block.bytes.length;
  if (cid.code === DAG_PB) {
    const node = dagPb.decode(block.bytes);
    for (const link of node.Links) {
      const child = CID.asCID(link.Hash);
      const childPath = link.Name ? (path ? `${path}/${link.Name}` : link.Name) : path;
      if (link.Name) entries.push(`${childPath} (${child.code === RAW ? "raw" : "dag-pb"}, Tsize ${link.Tsize ?? "?"})`);
      stack.push({ cid: child, path: childPath });
    }
  }
}
const extra = blocks.size - reached.size;
for (const e of entries) console.log(`  ${e}`);
console.log(`ok ${root} blocks=${reached.size} bytes=${bytesTotal}${extra ? ` extra-unreachable-blocks=${extra}` : ""}`);
