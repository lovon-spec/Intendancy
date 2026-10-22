import { describe, expect, it } from "vitest";
import { blockAtOrBefore, evidenceGroupID, evidenceLink, ipfsPath, readInChunks } from "./evidence";

describe("party evidence", () => {
  it("derives the evidence group the way the deployed registry does", () => {
    // RequestEvidenceGroupID events of the live registry 0x67DB…68f9, block 48,121,029,
    // tx 0x5c1f1451c545e3d0c29583e32261073620fd8d633f79fe62ba226f523eecf30a (request 0 of each item).
    expect(evidenceGroupID("0xa5fc5c83dcd18cd740aa5a4aaa578c011528a8720444ff3ae5af5ddcc5ccaa72", 0)).toBe(
      0x7fada92fc4f90ff6efd97a60fa990eb91894a4f629c239ca76ca5b06a8219db0n,
    );
    expect(evidenceGroupID("0xb4998ffb77797dd39f2e1aaccb1b0d0cb0b538a4dd0c3f93c9ca9998a33d3671", 0)).toBe(
      0x00894e75ba21eecd8084c2904454e3ad4a19ddc2eccb116310de8c8ec9d0e3a8n,
    );
    // Later requests, from `cast keccak $(cast abi-encode --packed '(bytes32,uint256)' <item> <index>)`.
    expect(evidenceGroupID("0xa5fc5c83dcd18cd740aa5a4aaa578c011528a8720444ff3ae5af5ddcc5ccaa72", 1)).toBe(
      0xe7999a70236e81fd3fec6d207970f4c1ead42fea79c0a084c9fc57bbf248345cn,
    );
    expect(evidenceGroupID("0xa5fc5c83dcd18cd740aa5a4aaa578c011528a8720444ff3ae5af5ddcc5ccaa72", 256n)).toBe(
      0x8ef31858838199109722c1978272a7f40feeff37a9b7b834134d61426345aef9n,
    );
  });

  it("reads logs in ranges a capped RPC accepts, halving until it does", async () => {
    const asked: [bigint, bigint][] = [];
    const events = [48_000_123n, 48_010_000n, 48_179_999n];
    // An RPC that refuses more than 10,000 blocks per call, like the public publicnode endpoint.
    const read = async (from: bigint, to: bigint) => {
      asked.push([from, to]);
      if (to - from + 1n > 10_000n) throw new Error("exceed maximum block range: 10000");
      return events.filter((block) => block >= from && block <= to);
    };
    const progress: bigint[] = [];
    const found = await readInChunks({ from: 48_000_000n, to: 48_180_000n }, read, { onProgress: (to) => progress.push(to) });
    expect(found).toEqual(events);
    const accepted = asked.filter(([from, to]) => to - from + 1n <= 10_000n);
    expect(asked.length - accepted.length).toBe(3);
    expect(accepted[0][0]).toBe(48_000_000n);
    expect(accepted.at(-1)?.[1]).toBe(48_180_000n);
    accepted.slice(1).forEach(([from], index) => expect(from).toBe(accepted[index][1] + 1n));
    expect(progress.at(-1)).toBe(48_180_000n);
    await expect(readInChunks({ from: 0n, to: 5_000n }, async () => { throw new Error("down"); })).rejects.toThrow("down");
  });

  it("starts the scan no later than the request, in one read on Gnosis' 5-second slots", async () => {
    // Every slot filled but a few, so blocks are 5 seconds apart or more.
    const stamp = (n: bigint) => 1_700_000_000n + n * 5n + (n > 900n ? 20n : 0n);
    const reads: (bigint | undefined)[] = [];
    const read = async (number?: bigint) => {
      reads.push(number);
      const n = number ?? 1_000n;
      return { number: n, timestamp: stamp(n) };
    };
    const { start, head } = await blockAtOrBefore(read, stamp(400n));
    expect(head).toBe(1_000n);
    expect(stamp(start)).toBeLessThanOrEqual(stamp(400n));
    expect(400n - start).toBeLessThanOrEqual(5n); // early by the 4 missed slots and the rounding block
    expect(reads).toHaveLength(2);
  });

  it("fetches only IPFS paths it can build safely, and links only IPFS or http(s)", () => {
    const cid = "QmUQZ1tRdbJ2wbTv7N3yjSuuEjNBSBWW6bcVhPdeC36Nqq";
    expect(ipfsPath(`/ipfs/${cid}/evidence.json`)).toBe(`${cid}/evidence.json`);
    expect(ipfsPath(`ipfs://${cid}`)).toBe(cid);
    expect(ipfsPath(`/ipfs/${cid}/../../api/v0/id`)).toBeNull();
    expect(ipfsPath("/ipfs/not-a-cid/evidence.json")).toBeNull();
    expect(ipfsPath("https://example.com/evidence.json")).toBeNull();
    expect(ipfsPath("The skill tells the agent to send keys away.")).toBeNull();
    expect(evidenceLink(`/ipfs/${cid}/report.pdf`, "https://dweb.link/")).toBe(`https://dweb.link/ipfs/${cid}/report.pdf`);
    expect(evidenceLink("https://example.com/a.pdf", "https://dweb.link")).toBe("https://example.com/a.pdf");
    expect(evidenceLink("javascript:alert(1)", "https://dweb.link")).toBeNull();
  });
});
