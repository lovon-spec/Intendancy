// In-browser verification of a skill tree fetched as a CAR, for reviewers.
//
// Mirrors the `intend` installer's checks in spirit (complete DAG, every block
// hash verified, codec and hash allowlist, no symlinks, the bounded UnixFS
// profile, the policy's size bound) but REPORTS findings instead of refusing:
// a juror decides by the policy, and needs to see what is wrong, not a revert.
// It is a review aid, not the installer; the CLI remains the reference.
import { CID } from "multiformats/cid";
// Pure-JS SHA-256: WebCrypto is unavailable outside secure contexts in Safari.
import { sha256 } from "@noble/hashes/sha2.js";
import * as dagPb from "@ipld/dag-pb";
import { CarReader } from "@ipld/car";
import { UnixFS } from "ipfs-unixfs";

export const CODEC_DAG_PB = 0x70;
export const CODEC_RAW = 0x55;
export const HASH_SHA256 = 0x12;

export interface TreeLimits {
  /** CAR bytes accepted before parsing (spec §9 transport-boundary reference value). */
  maxCarBytes: number;
  maxBlockBytes: number;
  maxBlocks: number;
  /** Listing policy criterion 9: the tree's total size. */
  maxTreeBytes: number;
  maxDepth: number;
  maxEntries: number;
  /** The installer profile's single-block raw leaf bound. */
  maxLeafBytes: number;
}

export const DEFAULT_LIMITS: TreeLimits = {
  maxCarBytes: 64 * 1024 * 1024,
  maxBlockBytes: 1024 * 1024,
  maxBlocks: 65_536,
  maxTreeBytes: 2 * 1024 * 1024,
  maxDepth: 32,
  maxEntries: 4096,
  maxLeafBytes: 256 * 1024,
};

export type IssueCode =
  | "CAR_TOO_LARGE"
  | "CAR_ROOTS"
  | "TOO_MANY_BLOCKS"
  | "BLOCK_SIZE"
  | "BAD_CID"
  | "CID_VERSION"
  | "HASH_MISMATCH"
  | "MISSING_BLOCK"
  | "CODEC"
  | "DECODE"
  | "SYMLINK"
  | "SHARD"
  | "UNIXFS_TYPE"
  | "NAME"
  | "ORDER"
  | "DUPLICATE"
  | "CHUNKING"
  | "LEAF_SIZE"
  | "DEPTH"
  | "ENTRIES"
  | "TREE_SIZE";

export interface TreeIssue {
  code: IssueCode;
  /** Tree path the finding is about; "" for the root. */
  path: string;
  message: string;
}

export interface TreeEntry {
  path: string;
  kind: "file" | "directory";
  size: number;
  cid: string;
  /** Blocks that carry this file's bytes; 1 for a raw leaf or inline data. */
  chunks: number;
}

export interface TreeStats {
  carBytes: number;
  blocks: number;
  reachableBlocks: number;
  unreachableBlocks: number;
  treeBytes: number;
  hasModes: boolean;
  hasMtimes: boolean;
}

/** Codes that mean the bytes cannot be trusted at all, as opposed to policy findings. */
export const INTEGRITY_CODES: ReadonlySet<IssueCode> = new Set<IssueCode>([
  "CAR_TOO_LARGE",
  "TOO_MANY_BLOCKS",
  "BLOCK_SIZE",
  "BAD_CID",
  "HASH_MISMATCH",
  "MISSING_BLOCK",
  "CODEC",
  "DECODE",
]);

export class VerifiedTree {
  readonly root: string;
  readonly entries: TreeEntry[];
  readonly issues: TreeIssue[];
  readonly stats: TreeStats;
  private readonly parts: Map<string, Uint8Array[]>;

  constructor(root: string, entries: TreeEntry[], issues: TreeIssue[], stats: TreeStats, parts: Map<string, Uint8Array[]>) {
    this.root = root;
    this.entries = entries;
    this.issues = issues;
    this.stats = stats;
    this.parts = parts;
  }

  /** True when nothing at all was found: complete, hash-verified, inside the profile and the policy bounds. */
  get ok(): boolean {
    return this.issues.length === 0;
  }

  /** True when the bytes are complete and hash-verified, whatever the policy findings. */
  get intact(): boolean {
    return !this.issues.some((issue) => INTEGRITY_CODES.has(issue.code));
  }

  has(path: string): boolean {
    return this.parts.has(path);
  }

  /** The complete bytes of a file, reassembled from its verified blocks. */
  read(path: string): Uint8Array {
    const parts = this.parts.get(path);
    if (!parts) throw new Error(`no file at ${path}`);
    if (parts.length === 1) return parts[0];
    const total = parts.reduce((sum, part) => sum + part.length, 0);
    const out = new Uint8Array(total);
    let offset = 0;
    for (const part of parts) {
      out.set(part, offset);
      offset += part.length;
    }
    return out;
  }
}

/** Parse a Tree CID string and enforce the policy form: CIDv1, DAG-PB, 32-byte SHA-256. */
export function parseTreeCid(value: string): CID {
  let cid: CID;
  try {
    cid = CID.parse(value);
  } catch (caught) {
    throw new Error(`Tree CID does not parse: ${caught instanceof Error ? caught.message : String(caught)}`);
  }
  if (cid.version !== 1) throw new Error("Tree CID must be CIDv1");
  if (cid.code !== CODEC_DAG_PB) throw new Error("Tree CID must use the DAG-PB codec");
  if (cid.multihash.code !== HASH_SHA256 || cid.multihash.digest.length !== 32) {
    throw new Error("Tree CID must carry a 32-byte SHA-256 multihash");
  }
  if (cid.toString() !== value) throw new Error("Tree CID must be in canonical base32 form");
  return cid;
}

function blockKey(cid: CID): string {
  return `${cid.code}:${toHex(cid.multihash.digest)}`;
}

function toHex(bytes: Uint8Array): string {
  let out = "";
  for (const byte of bytes) out += byte.toString(16).padStart(2, "0");
  return out;
}

function equalBytes(a: Uint8Array, b: Uint8Array): boolean {
  if (a.length !== b.length) return false;
  for (let index = 0; index < a.length; index += 1) if (a[index] !== b[index]) return false;
  return true;
}

/** Strict code-point order, which equals UTF-8 byte order. */
function compareNames(a: string, b: string): number {
  const ia = a[Symbol.iterator]();
  const ib = b[Symbol.iterator]();
  for (;;) {
    const na = ia.next();
    const nb = ib.next();
    if (na.done && nb.done) return 0;
    if (na.done) return -1;
    if (nb.done) return 1;
    const ca = na.value.codePointAt(0) ?? 0;
    const cb = nb.value.codePointAt(0) ?? 0;
    if (ca !== cb) return ca < cb ? -1 : 1;
  }
}

function joinPath(base: string, name: string): string {
  return base === "" ? name : `${base}/${name}`;
}

/**
 * Whether the UnixFS Data message carries the optional mode (field 7) or mtime
 * (field 8). The UnixFS library fills in defaults on unmarshal, so presence has
 * to be read from the protobuf itself; this walks the top-level fields only.
 */
export function unixfsMetadataPresence(data: Uint8Array): { mode: boolean; mtime: boolean } {
  let mode = false;
  let mtime = false;
  let index = 0;
  const varint = (): number | null => {
    let value = 0;
    let shift = 0;
    while (index < data.length) {
      const byte = data[index];
      index += 1;
      value += (byte & 0x7f) * 2 ** shift;
      if ((byte & 0x80) === 0) return value;
      shift += 7;
      if (shift > 63) return null;
    }
    return null;
  };
  while (index < data.length) {
    const tag = varint();
    if (tag === null) break;
    const field = Math.floor(tag / 8);
    const wire = tag % 8;
    if (field === 7) mode = true;
    if (field === 8) mtime = true;
    if (wire === 0) {
      if (varint() === null) break;
    } else if (wire === 1) {
      index += 8;
    } else if (wire === 2) {
      const length = varint();
      if (length === null) break;
      index += length;
    } else if (wire === 5) {
      index += 4;
    } else {
      break;
    }
  }
  return { mode, mtime };
}

/** Text if it decodes as UTF-8 without error and carries no NUL byte. */
export function isProbablyText(bytes: Uint8Array): boolean {
  if (bytes.some((byte) => byte === 0)) return false;
  try {
    new TextDecoder("utf-8", { fatal: true }).decode(bytes);
    return true;
  } catch {
    return false;
  }
}

export async function verifyTreeCar(
  car: Uint8Array,
  expectedRoot: string,
  limits: TreeLimits = DEFAULT_LIMITS,
): Promise<VerifiedTree> {
  const issues: TreeIssue[] = [];
  const root = parseTreeCid(expectedRoot);
  const stats: TreeStats = {
    carBytes: car.length,
    blocks: 0,
    reachableBlocks: 0,
    unreachableBlocks: 0,
    treeBytes: 0,
    hasModes: false,
    hasMtimes: false,
  };
  const entries: TreeEntry[] = [];
  const parts = new Map<string, Uint8Array[]>();

  if (car.length > limits.maxCarBytes) {
    issues.push({ code: "CAR_TOO_LARGE", path: "", message: `CAR is ${car.length} bytes, above the ${limits.maxCarBytes} byte bound` });
    return new VerifiedTree(expectedRoot, entries, issues, stats, parts);
  }

  const reader = await CarReader.fromBytes(car);
  const roots = await reader.getRoots();
  if (!roots.some((candidate) => blockKey(candidate) === blockKey(root))) {
    issues.push({ code: "CAR_ROOTS", path: "", message: "the CAR header does not name the Tree CID as a root" });
  }

  const blocks = new Map<string, Uint8Array>();
  for await (const { cid, bytes } of reader.blocks()) {
    stats.blocks += 1;
    if (stats.blocks > limits.maxBlocks) {
      issues.push({ code: "TOO_MANY_BLOCKS", path: "", message: `more than ${limits.maxBlocks} blocks; the rest were not read` });
      break;
    }
    if (bytes.length > limits.maxBlockBytes) {
      issues.push({ code: "BLOCK_SIZE", path: "", message: `block ${cid} is ${bytes.length} bytes, above the ${limits.maxBlockBytes} byte bound` });
      continue;
    }
    if (cid.multihash.code !== HASH_SHA256 || cid.multihash.digest.length !== 32) {
      issues.push({ code: "BAD_CID", path: "", message: `block ${cid} does not use a 32-byte SHA-256 multihash` });
      continue;
    }
    const digest = sha256(bytes);
    if (!equalBytes(digest, cid.multihash.digest)) {
      issues.push({ code: "HASH_MISMATCH", path: "", message: `block ${cid} does not hash to its CID` });
      continue;
    }
    blocks.set(blockKey(cid), bytes);
  }

  const reachable = new Set<string>();

  const getBlock = (cid: CID, path: string): Uint8Array | undefined => {
    const bytes = blocks.get(blockKey(cid));
    if (!bytes) {
      issues.push({ code: "MISSING_BLOCK", path, message: `block ${cid} for ${path || "the root"} is not in the CAR` });
      return undefined;
    }
    reachable.add(blockKey(cid));
    return bytes;
  };

  const checkLinkCid = (cid: CID, path: string): boolean => {
    let good = true;
    if (cid.version !== 1) {
      // A policy finding, not an integrity failure: the block is still looked up
      // by codec and digest, so the tree renders and the juror sees the finding.
      issues.push({ code: "CID_VERSION", path, message: `${path || "the root"} is linked by a CIDv0; the policy requires CIDv1 for every linked CID` });
    }
    if (cid.code !== CODEC_DAG_PB && cid.code !== CODEC_RAW) {
      issues.push({ code: "CODEC", path, message: `${path || "the root"} uses codec 0x${cid.code.toString(16)}; only DAG-PB and raw are allowed` });
      good = false;
    }
    if (cid.multihash.code !== HASH_SHA256 || cid.multihash.digest.length !== 32) {
      issues.push({ code: "BAD_CID", path, message: `${path || "the root"} does not use a 32-byte SHA-256 multihash` });
      good = false;
    }
    return good;
  };

  const addFile = (path: string, cid: CID, filePartsList: Uint8Array[]) => {
    const size = filePartsList.reduce((sum, part) => sum + part.length, 0);
    stats.treeBytes += size;
    entries.push({ path, kind: "file", size, cid: cid.toString(), chunks: filePartsList.length });
    parts.set(path, filePartsList);
  };

  const noteEntry = (): boolean => {
    if (entries.length >= limits.maxEntries) {
      issues.push({ code: "ENTRIES", path: "", message: `more than ${limits.maxEntries} entries; the rest were not walked` });
      return false;
    }
    return true;
  };

  // A chunk is a raw leaf, or (when the producer disabled raw leaves, as kubo
  // does with --preserve-mode) a DAG-PB file or raw node carrying the bytes in
  // its Data field and linking nothing. Anything with links is a deeper tree,
  // outside the installer profile.
  const readChunk = (cid: CID, chunkPath: string, path: string, index: number): Uint8Array | undefined => {
    const bytes = getBlock(cid, chunkPath);
    if (!bytes) return undefined;
    if (cid.code === CODEC_RAW) return bytes;
    let node: dagPb.PBNode;
    let fs: UnixFS;
    try {
      node = dagPb.decode(bytes);
      fs = UnixFS.unmarshal(node.Data ?? new Uint8Array());
    } catch (caught) {
      issues.push({ code: "DECODE", path: chunkPath, message: `${path} chunk ${index} is not a valid UnixFS node: ${caught instanceof Error ? caught.message : String(caught)}` });
      return undefined;
    }
    const chunkMeta = unixfsMetadataPresence(node.Data ?? new Uint8Array());
    if (chunkMeta.mode) stats.hasModes = true;
    if (chunkMeta.mtime) stats.hasMtimes = true;
    if ((fs.type !== "file" && fs.type !== "raw") || node.Links.length > 0) {
      issues.push({ code: "CHUNKING", path, message: `${path} chunk ${index} is a ${fs.type} node with ${node.Links.length} links; multi-level file trees are outside the installer profile` });
      return undefined;
    }
    return fs.data ?? new Uint8Array();
  };

  const visitFile = (cid: CID, path: string, node: dagPb.PBNode, fs: UnixFS) => {
    const fileParts: Uint8Array[] = [];
    if (node.Links.length === 0) {
      // Inline data: the whole file lives in the node's Data field.
      fileParts.push(fs.data ?? new Uint8Array());
    } else {
      if (fs.blockSizes.length !== node.Links.length) {
        issues.push({ code: "CHUNKING", path, message: `${path} declares ${fs.blockSizes.length} block sizes for ${node.Links.length} chunks` });
      }
      if (fs.data && fs.data.length > 0) {
        issues.push({ code: "CHUNKING", path, message: `${path} mixes inline data with chunk links; outside the installer profile` });
      }
      node.Links.forEach((link, index) => {
        const chunkPath = `${path}#${index}`;
        if (!checkLinkCid(link.Hash, chunkPath)) return;
        const chunk = readChunk(link.Hash, chunkPath, path, index);
        if (!chunk) return;
        if (chunk.length > limits.maxLeafBytes) {
          issues.push({ code: "LEAF_SIZE", path, message: `${path} chunk ${index} is ${chunk.length} bytes, above the ${limits.maxLeafBytes} byte leaf bound` });
        }
        const declared = fs.blockSizes[index];
        if (declared !== undefined && BigInt(chunk.length) !== declared) {
          issues.push({ code: "CHUNKING", path, message: `${path} chunk ${index} is ${chunk.length} bytes but declares ${declared}` });
        }
        fileParts.push(chunk);
      });
    }
    addFile(path, cid, fileParts);
  };

  const visit = (cid: CID, path: string, depth: number) => {
    if (depth > limits.maxDepth) {
      issues.push({ code: "DEPTH", path, message: `${path} is deeper than ${limits.maxDepth} levels` });
      return;
    }
    if (!checkLinkCid(cid, path)) return;
    const bytes = getBlock(cid, path);
    if (!bytes) return;
    if (!noteEntry()) return;

    if (cid.code === CODEC_RAW) {
      if (bytes.length > limits.maxLeafBytes) {
        issues.push({ code: "LEAF_SIZE", path, message: `${path} is a ${bytes.length} byte raw leaf, above the ${limits.maxLeafBytes} byte bound` });
      }
      addFile(path, cid, [bytes]);
      return;
    }

    let node: dagPb.PBNode;
    try {
      node = dagPb.decode(bytes);
    } catch (caught) {
      issues.push({ code: "DECODE", path, message: `${path || "the root"} is not a valid DAG-PB node: ${caught instanceof Error ? caught.message : String(caught)}` });
      return;
    }
    let fs: UnixFS;
    try {
      fs = UnixFS.unmarshal(node.Data ?? new Uint8Array());
    } catch (caught) {
      issues.push({ code: "DECODE", path, message: `${path || "the root"} carries no valid UnixFS data: ${caught instanceof Error ? caught.message : String(caught)}` });
      return;
    }
    const meta = unixfsMetadataPresence(node.Data ?? new Uint8Array());
    if (meta.mode) stats.hasModes = true;
    if (meta.mtime) stats.hasMtimes = true;

    switch (fs.type) {
      case "directory": {
        if (path === "" && depth !== 0) break;
        entries.push({ path, kind: "directory", size: 0, cid: cid.toString(), chunks: 0 });
        const seen = new Set<string>();
        const seenFolded = new Set<string>();
        let previous: string | undefined;
        for (const link of node.Links) {
          const name = link.Name ?? "";
          const childPath = joinPath(path, name);
          if (name === "" || name === "." || name === ".." || name.includes("/") || name.includes("\0")) {
            issues.push({ code: "NAME", path: childPath, message: `directory ${path || "/"} has an entry with an unusable name ${JSON.stringify(name)}` });
            continue;
          }
          if (seen.has(name)) {
            issues.push({ code: "DUPLICATE", path: childPath, message: `directory ${path || "/"} names ${JSON.stringify(name)} twice` });
            continue;
          }
          const folded = name.normalize("NFC").toLowerCase();
          if (seenFolded.has(folded)) {
            issues.push({ code: "DUPLICATE", path: childPath, message: `directory ${path || "/"} has names that collide on a case-insensitive or Unicode-normalizing filesystem: ${JSON.stringify(name)}` });
          }
          if (previous !== undefined && compareNames(previous, name) >= 0) {
            issues.push({ code: "ORDER", path: childPath, message: `directory ${path || "/"} links are not strictly sorted at ${JSON.stringify(name)}` });
          }
          seen.add(name);
          seenFolded.add(folded);
          previous = name;
          visit(link.Hash, childPath, depth + 1);
        }
        break;
      }
      case "file":
      case "raw":
        if (path === "") {
          issues.push({ code: "UNIXFS_TYPE", path, message: "the Tree CID is a file, not a directory" });
          break;
        }
        visitFile(cid, path, node, fs);
        break;
      case "symlink":
        issues.push({ code: "SYMLINK", path, message: `${path} is a symlink; the policy forbids symlinks in trees` });
        break;
      case "hamt-sharded-directory":
        issues.push({ code: "SHARD", path, message: `${path || "the root"} is a HAMT-sharded directory; outside the installer profile` });
        break;
      default:
        issues.push({ code: "UNIXFS_TYPE", path, message: `${path || "the root"} has UnixFS type ${fs.type}, which the profile does not accept` });
    }
  };

  visit(root, "", 0);

  if (stats.treeBytes > limits.maxTreeBytes) {
    issues.push({ code: "TREE_SIZE", path: "", message: `the tree is ${stats.treeBytes} bytes; the policy bound is ${limits.maxTreeBytes}` });
  }
  stats.reachableBlocks = reachable.size;
  stats.unreachableBlocks = blocks.size - reachable.size;
  entries.sort((a, b) => compareNames(a.path, b.path));
  return new VerifiedTree(expectedRoot, entries, issues, stats, parts);
}

export const DEFAULT_GATEWAYS = ["https://dweb.link", "https://ipfs.io", "https://trustless-gateway.link"];

export interface FetchedCar {
  bytes: Uint8Array;
  gateway: string;
}

/** Fetch the complete DAG under a CID as a CAR from the first gateway that answers, capped at `maxBytes`. */
export async function fetchTreeCar(
  cid: string,
  gateways: string[] = DEFAULT_GATEWAYS,
  maxBytes: number = DEFAULT_LIMITS.maxCarBytes,
  signal?: AbortSignal,
): Promise<FetchedCar> {
  const failures: string[] = [];
  for (const gateway of gateways) {
    const url = `${gateway.replace(/\/$/, "")}/ipfs/${encodeURIComponent(cid)}?format=car&dag-scope=all`;
    try {
      const response = await fetch(url, { headers: { Accept: "application/vnd.ipld.car" }, signal });
      if (!response.ok) {
        failures.push(`${gateway}: HTTP ${response.status}`);
        continue;
      }
      const declared = Number(response.headers.get("content-length") ?? "0");
      if (declared > maxBytes) {
        failures.push(`${gateway}: declares ${declared} bytes, above the ${maxBytes} byte bound`);
        continue;
      }
      const bytes = await readCapped(response, maxBytes);
      if (!bytes) {
        failures.push(`${gateway}: body exceeded the ${maxBytes} byte bound`);
        continue;
      }
      return { bytes, gateway };
    } catch (caught) {
      failures.push(`${gateway}: ${caught instanceof Error ? caught.message : String(caught)}`);
    }
  }
  throw new Error(`no gateway delivered the CAR: ${failures.join("; ")}`);
}

async function readCapped(response: Response, maxBytes: number): Promise<Uint8Array | null> {
  if (!response.body) {
    const buffer = new Uint8Array(await response.arrayBuffer());
    return buffer.length > maxBytes ? null : buffer;
  }
  const reader = response.body.getReader();
  const chunks: Uint8Array[] = [];
  let total = 0;
  for (;;) {
    const { done, value } = await reader.read();
    if (done) break;
    total += value.length;
    if (total > maxBytes) {
      await reader.cancel();
      return null;
    }
    chunks.push(value);
  }
  const out = new Uint8Array(total);
  let offset = 0;
  for (const chunk of chunks) {
    out.set(chunk, offset);
    offset += chunk.length;
  }
  return out;
}
