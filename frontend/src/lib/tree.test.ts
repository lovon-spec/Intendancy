import { describe, expect, it } from "vitest";
import { readFileSync, readdirSync, statSync } from "node:fs";
import { join } from "node:path";
import { fileURLToPath } from "node:url";
import { CID } from "multiformats/cid";
import { sha256 } from "multiformats/hashes/sha2";
import * as raw from "multiformats/codecs/raw";
import * as dagPb from "@ipld/dag-pb";
import { CarWriter } from "@ipld/car";
import { UnixFS } from "ipfs-unixfs";
import { DEFAULT_LIMITS, parseTreeCid, verifyTreeCar } from "./tree";

const HERE = fileURLToPath(new URL(".", import.meta.url));
const FIXTURES = join(HERE, "../../../cli/fixtures/kubo");
const ROOT = readFileSync(join(FIXTURES, "root-cid.txt"), "utf8").trim();
const MODE_ROOT = readFileSync(join(FIXTURES, "mode-root-cid.txt"), "utf8").trim();
const CAR = new Uint8Array(readFileSync(join(FIXTURES, "tree.car")));
const MODE_CAR = new Uint8Array(readFileSync(join(FIXTURES, "tree-mode.car")));

function diskFiles(dir: string, base = ""): Map<string, Uint8Array> {
  const out = new Map<string, Uint8Array>();
  for (const name of readdirSync(dir)) {
    const full = join(dir, name);
    const rel = base ? `${base}/${name}` : name;
    if (statSync(full).isDirectory()) {
      for (const [path, bytes] of diskFiles(full, rel)) out.set(path, bytes);
    } else {
      out.set(rel, new Uint8Array(readFileSync(full)));
    }
  }
  return out;
}

async function rawBlock(bytes: Uint8Array) {
  return { cid: CID.createV1(raw.code, await sha256.digest(bytes)), bytes };
}

async function pbBlock(node: dagPb.PBNode) {
  const bytes = dagPb.encode(node);
  return { cid: CID.createV1(dagPb.code, await sha256.digest(bytes)), bytes };
}

async function carOf(root: CID, blocks: { cid: CID; bytes: Uint8Array }[]): Promise<Uint8Array> {
  const { writer, out } = CarWriter.create([root]);
  const chunks: Uint8Array[] = [];
  const drained = (async () => {
    for await (const chunk of out) chunks.push(chunk);
  })();
  for (const block of blocks) await writer.put(block);
  await writer.close();
  await drained;
  const total = chunks.reduce((sum, chunk) => sum + chunk.length, 0);
  const bytes = new Uint8Array(total);
  let offset = 0;
  for (const chunk of chunks) {
    bytes.set(chunk, offset);
    offset += chunk.length;
  }
  return bytes;
}

describe("kubo interop vectors", () => {
  it("verifies the default vector completely and reproduces every file", async () => {
    const tree = await verifyTreeCar(CAR, ROOT);
    expect(tree.issues).toEqual([]);
    expect(tree.ok).toBe(true);
    expect(tree.stats.unreachableBlocks).toBe(0);
    expect(tree.stats.hasModes).toBe(false);
    expect(tree.stats.hasMtimes).toBe(false);
    const disk = diskFiles(join(FIXTURES, "tree"));
    const files = tree.entries.filter((entry) => entry.kind === "file");
    expect(files.map((entry) => entry.path).sort()).toEqual([...disk.keys()].sort());
    for (const [path, bytes] of disk) {
      expect(tree.has(path)).toBe(true);
      expect(Buffer.from(tree.read(path)).equals(Buffer.from(bytes))).toBe(true);
    }
    const chunky = files.find((entry) => entry.path === "references/chunky.bin");
    expect(chunky?.chunks).toBeGreaterThan(1);
    expect(chunky?.size).toBe(disk.get("references/chunky.bin")?.length);
    expect(tree.stats.treeBytes).toBe([...disk.values()].reduce((sum, bytes) => sum + bytes.length, 0));
  });

  it("verifies the metadata vector and reports modes and mtimes as present", async () => {
    const tree = await verifyTreeCar(MODE_CAR, MODE_ROOT);
    expect(tree.issues).toEqual([]);
    expect(tree.stats.hasModes).toBe(true);
    expect(tree.stats.hasMtimes).toBe(true);
    expect(Buffer.from(tree.read("SKILL.md")).equals(readFileSync(join(FIXTURES, "tree/SKILL.md")))).toBe(true);
  });

  it("reports a root that the CAR does not contain", async () => {
    const tree = await verifyTreeCar(MODE_CAR, ROOT);
    expect(tree.intact).toBe(false);
    expect(tree.issues.map((issue) => issue.code)).toContain("MISSING_BLOCK");
    expect(tree.entries).toEqual([]);
  });

  it("detects a flipped byte inside a block", async () => {
    const needle = Buffer.from(readFileSync(join(FIXTURES, "tree/references/a.md")));
    const offset = Buffer.from(CAR).indexOf(needle);
    expect(offset).toBeGreaterThan(0);
    const tampered = new Uint8Array(CAR);
    tampered[offset] ^= 0x01;
    const tree = await verifyTreeCar(tampered, ROOT);
    expect(tree.intact).toBe(false);
    expect(tree.issues.map((issue) => issue.code)).toContain("HASH_MISMATCH");
    expect(tree.has("references/a.md")).toBe(false);
  });

  it("does not trust a truncated CAR", async () => {
    const truncated = CAR.slice(0, Math.floor(CAR.length / 2));
    const tree = await verifyTreeCar(truncated, ROOT).catch(() => null);
    if (tree) expect(tree.intact).toBe(false);
  });

  it("applies the CAR size bound before parsing", async () => {
    const tree = await verifyTreeCar(CAR, ROOT, { ...DEFAULT_LIMITS, maxCarBytes: 1024 });
    expect(tree.issues[0]?.code).toBe("CAR_TOO_LARGE");
    expect(tree.intact).toBe(false);
  });

  it("flags the policy size bound without losing the bytes", async () => {
    const tree = await verifyTreeCar(CAR, ROOT, { ...DEFAULT_LIMITS, maxTreeBytes: 1024 });
    expect(tree.intact).toBe(true);
    expect(tree.ok).toBe(false);
    expect(tree.issues.map((issue) => issue.code)).toContain("TREE_SIZE");
  });
});

describe("tree CID form", () => {
  it("accepts the canonical form and rejects the others", () => {
    expect(parseTreeCid(ROOT).toString()).toBe(ROOT);
    expect(() => parseTreeCid("QmYwAPJzv5CZsnA625s3Xf2nemtYgPpHdWEz79ojWnPbdG")).toThrow(/CIDv1/);
    expect(() => parseTreeCid(ROOT.toUpperCase())).toThrow();
    expect(() => parseTreeCid("not a cid")).toThrow(/parse/);
  });
});

describe("synthetic trees", () => {
  it("reports a symlink and a slash in a name, and still reads the good file", async () => {
    const skill = await rawBlock(new TextEncoder().encode("---\nname: x\ndescription: y\n---\n"));
    const link = await pbBlock({
      Data: new UnixFS({ type: "symlink", data: new TextEncoder().encode("../etc/passwd") }).marshal(),
      Links: [],
    });
    const dir = await pbBlock({
      Data: new UnixFS({ type: "directory" }).marshal(),
      Links: [
        { Name: "SKILL.md", Tsize: skill.bytes.length, Hash: skill.cid },
        { Name: "bad/name", Tsize: skill.bytes.length, Hash: skill.cid },
        { Name: "link", Tsize: link.bytes.length, Hash: link.cid },
      ],
    });
    const car = await carOf(dir.cid, [dir, skill, link]);
    const tree = await verifyTreeCar(car, dir.cid.toString());
    const codes = tree.issues.map((issue) => issue.code);
    expect(codes).toContain("SYMLINK");
    expect(codes).toContain("NAME");
    expect(tree.intact).toBe(true);
    expect(tree.ok).toBe(false);
    expect(new TextDecoder().decode(tree.read("SKILL.md"))).toContain("name: x");
  });

  it("reassembles a chunked file and checks its declared block sizes", async () => {
    const a = await rawBlock(new Uint8Array([1, 2, 3]));
    const b = await rawBlock(new Uint8Array([4, 5]));
    const good = await pbBlock({
      Data: new UnixFS({ type: "file", blockSizes: [3n, 2n] }).marshal(),
      Links: [
        { Name: "", Tsize: 3, Hash: a.cid },
        { Name: "", Tsize: 2, Hash: b.cid },
      ],
    });
    const lying = await pbBlock({
      Data: new UnixFS({ type: "file", blockSizes: [3n, 9n] }).marshal(),
      Links: [
        { Name: "", Tsize: 3, Hash: a.cid },
        { Name: "", Tsize: 2, Hash: b.cid },
      ],
    });
    const dir = await pbBlock({
      Data: new UnixFS({ type: "directory" }).marshal(),
      Links: [
        { Name: "good.bin", Tsize: 5, Hash: good.cid },
        { Name: "lying.bin", Tsize: 5, Hash: lying.cid },
      ],
    });
    const car = await carOf(dir.cid, [dir, good, lying, a, b]);
    const tree = await verifyTreeCar(car, dir.cid.toString());
    expect([...tree.read("good.bin")]).toEqual([1, 2, 3, 4, 5]);
    expect(tree.entries.find((entry) => entry.path === "good.bin")?.chunks).toBe(2);
    expect(tree.issues.some((issue) => issue.code === "CHUNKING" && issue.path === "lying.bin")).toBe(true);
  });

  it("rejects a file as the tree root", async () => {
    const leaf = await rawBlock(new Uint8Array([7]));
    const file = await pbBlock({
      Data: new UnixFS({ type: "file", blockSizes: [1n] }).marshal(),
      Links: [{ Name: "", Tsize: 1, Hash: leaf.cid }],
    });
    const car = await carOf(file.cid, [file, leaf]);
    const tree = await verifyTreeCar(car, file.cid.toString());
    expect(tree.issues.map((issue) => issue.code)).toContain("UNIXFS_TYPE");
  });
});
