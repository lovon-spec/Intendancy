// Generates fixtures/descriptor-vectors.json: independent canonical RLP vectors for
// the V1 six-column descriptor, produced OUTSIDE the Rust crate so the Rust encoder's
// round-trip is checked against a second implementation lineage.
//
// Sources, in preference order per vector:
//   "frontend-encoder" — the repo's real frontend/src/lib/encoder.ts (viem toRlp +
//                        stringToHex + the frontend's own validation), loaded via
//                        node's native TypeScript type stripping;
//   "viem-toRlp"       — raw viem toRlp over stringToHex columns, for edge cases the
//                        frontend validation intentionally refuses (they must still
//                        DECODE identically on the Rust side; screening is separate).
//
// Run (no arguments; paths are resolved relative to this file):
//   node tools/gen-descriptor-vectors.mjs
//
// The output file is committed; regeneration is only needed if the descriptor
// schema, the frontend encoder, or viem's RLP ever change.

import { createRequire, registerHooks } from "node:module";
import { createHash } from "node:crypto";
import { writeFileSync } from "node:fs";
import path from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

// The frontend uses bundler-style extensionless relative imports ("./schema");
// node's ESM loader wants explicit extensions. Retry with ".ts" appended so the
// REAL frontend sources load in place, unmodified.
registerHooks({
  resolve(specifier, context, nextResolve) {
    try {
      return nextResolve(specifier, context);
    } catch (err) {
      if (
        (specifier.startsWith("./") || specifier.startsWith("../")) &&
        !specifier.endsWith(".ts")
      ) {
        return nextResolve(`${specifier}.ts`, context);
      }
      throw err;
    }
  },
});

const here = path.dirname(fileURLToPath(import.meta.url));
const frontend = path.resolve(here, "../../../frontend");
const outPath = path.resolve(here, "../fixtures/descriptor-vectors.json");

const require_ = createRequire(path.join(frontend, "package.json"));
const viemNs = await import(pathToFileURL(require_.resolve("viem")));
const viem = viemNs.default ?? viemNs;
const { toRlp, stringToHex, keccak256 } = viem;
const viemVersion = require_("viem/package.json").version;

const encoderMod = await import(
  pathToFileURL(path.join(frontend, "src/lib/encoder.ts"))
);

// Independent canonical Tree CID (CIDv1, dag-pb 0x70, sha2-256/32, base32 lower,
// no padding) — implemented here from scratch so even the CID text form has a
// second lineage.
const B32 = "abcdefghijklmnopqrstuvwxyz234567";
function base32NoPad(bytes) {
  let out = "";
  let buffer = 0;
  let bits = 0;
  for (const b of bytes) {
    buffer = (buffer << 8) | b;
    bits += 8;
    while (bits >= 5) {
      bits -= 5;
      out += B32[(buffer >> bits) & 31];
    }
  }
  if (bits > 0) out += B32[(buffer << (5 - bits)) & 31];
  return out;
}
function canonicalCid(content) {
  const digest = createHash("sha256").update(content).digest();
  const bytes = Buffer.concat([Buffer.from([0x01, 0x70, 0x12, 0x20]), digest]);
  return "b" + base32NoPad(bytes);
}

const ORDER = ["name", "description", "treeCid", "runtimes", "origin", "reserved"];

function rawEncode(fields) {
  return toRlp(ORDER.map((k) => stringToHex(fields[k])));
}

const cases = [
  {
    label: "realistic-skill",
    fields: {
      name: "vector-skill-one",
      description: "A cross-implementation descriptor vector for the Gate 2 bench.",
      treeCid: canonicalCid("vector-tree-one"),
      runtimes: "generic",
      origin: "",
      reserved: "",
    },
  },
  {
    label: "origin-bound",
    fields: {
      name: "vector-skill-two",
      description: "Vector with a populated Origin column.",
      treeCid: canonicalCid("vector-tree-two"),
      runtimes: "claude-code,generic",
      origin: "https://example.org/repo#" + canonicalCid("origin-pin"),
      reserved: "",
    },
  },
  {
    label: "all-empty",
    fields: {
      name: "",
      description: "",
      treeCid: "",
      runtimes: "",
      origin: "",
      reserved: "",
    },
  },
  {
    label: "unicode",
    fields: {
      name: "vector-unicode",
      description: "Émoji 🛠️ and CJK 漢字 and combining á must byte-match.",
      treeCid: canonicalCid("vector-tree-unicode"),
      runtimes: "generic",
      origin: "",
      reserved: "",
    },
  },
  {
    label: "long-description",
    fields: {
      name: "vector-long",
      description: "Long " + "x".repeat(500),
      treeCid: canonicalCid("vector-tree-long"),
      runtimes: "generic",
      origin: "",
      reserved: "",
    },
  },
  {
    label: "single-chars",
    fields: {
      name: "a",
      description: "b",
      treeCid: "c",
      runtimes: "d",
      origin: "e",
      reserved: "",
    },
  },
  {
    label: "reserved-nonempty-encoding-only",
    // Screening must REJECT this; the encoding itself is still well-defined and the
    // Rust codec must round-trip it so the rejection happens at the policy layer.
    fields: {
      name: "vector-reserved",
      description: "Reserved column populated (policy-invalid, codec-valid).",
      treeCid: canonicalCid("vector-tree-reserved"),
      runtimes: "generic",
      origin: "",
      reserved: "v2-not-allowed",
    },
  },
];

const vectors = [];
for (const c of cases) {
  let source = "viem-toRlp";
  let rlp;
  try {
    rlp = encoderMod.encodeItem(c.fields);
    source = "frontend-encoder";
    // Cross-check: the frontend decode must invert it.
    const back = encoderMod.decodeItem(rlp);
    for (const k of ORDER) {
      if (back[k] !== c.fields[k]) {
        throw new Error(`frontend decode mismatch on ${k}`);
      }
    }
  } catch {
    rlp = rawEncode(c.fields);
  }
  // Whatever the source, raw viem toRlp must agree byte-for-byte.
  if (rawEncode(c.fields) !== rlp) {
    throw new Error(`encoder disagreement on ${c.label}`);
  }
  vectors.push({
    label: c.label,
    source,
    columns: ORDER.map((k) => c.fields[k]),
    rlpHex: rlp,
    itemIdHex: keccak256(rlp),
  });
}

const out = {
  generator: "spikes/snapshot-bench/tools/gen-descriptor-vectors.mjs",
  encoderModule: "frontend/src/lib/encoder.ts",
  viemVersion,
  node: process.version,
  vectors,
};
writeFileSync(outPath, JSON.stringify(out, null, 2) + "\n");
console.log(
  `wrote ${vectors.length} vectors (${vectors.filter((v) => v.source === "frontend-encoder").length} via frontend encoder) to ${outPath}`
);
