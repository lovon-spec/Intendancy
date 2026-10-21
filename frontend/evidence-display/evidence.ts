// Party evidence filed on the registry: `submitEvidence`, and the reason
// given with `challengeRequest` or `removeItem`. The court lists only the
// Evidence events its arbitrable emits. When the case comes through an
// Intendment wrapper, that arbitrable is the wrapper, which emits none, so
// the display reads them from the registry itself.
import { CID } from "multiformats/cid";
import { encodePacked, keccak256, type Address, type Hex, type PublicClient } from "viem";
import { generalizedTcrAbi } from "../src/abi/GeneralizedTCR";
import { readCapped } from "../src/lib/tree";

/**
 * A request's evidence group in Classic Curate, `uint(keccak256(abi.encodePacked(_itemID, requestIndex)))`:
 * kleros/tcr contracts/GeneralizedTCR.sol @ 72e547ea135d839dc5db34e79e9f94f05c6a92bb, L492 in
 * submitEvidence. challengeRequest (L314), removeItem (L271) and requestStatusChange (L600) derive
 * the same value for the same request.
 */
export function evidenceGroupID(itemID: Hex, requestIndex: number | bigint): bigint {
  return BigInt(keccak256(encodePacked(["bytes32", "uint256"], [itemID, BigInt(requestIndex)])));
}

export interface BlockRange {
  from: bigint;
  to: bigint;
}

/** The first chunk, in blocks; halving reaches 10,000, a common public cap, after three failures. */
export const DEFAULT_SPAN = 80_000n;
export const MIN_SPAN = 1_250n;
/** About 115 days of Gnosis blocks: past any request's challenge, concession window and court rounds. */
export const MAX_SCAN_BLOCKS = 2_000_000n;

/**
 * Read `range` in consecutive chunks of at most `span` blocks, so RPCs that cap getLogs ranges
 * accept every call. A failed chunk is retried at half the span; a failure at `minSpan` is final.
 */
export async function readInChunks<T>(
  range: BlockRange,
  read: (from: bigint, to: bigint) => Promise<T[]>,
  options: { span?: bigint; minSpan?: bigint; onProgress?: (scannedTo: bigint) => void; signal?: AbortSignal } = {},
): Promise<T[]> {
  const minSpan = options.minSpan ?? MIN_SPAN;
  let span = options.span ?? DEFAULT_SPAN;
  const found: T[] = [];
  let from = range.from;
  while (from <= range.to) {
    options.signal?.throwIfAborted();
    const to = from + span - 1n < range.to ? from + span - 1n : range.to;
    try {
      found.push(...(await read(from, to)));
    } catch (error) {
      if (options.signal?.aborted || span <= minSpan) throw error;
      span = span / 2n > minSpan ? span / 2n : minSpan;
      continue;
    }
    options.onProgress?.(to);
    from = to + 1n;
  }
  return found;
}

export interface BlockStamp {
  number: bigint;
  timestamp: bigint;
}

/**
 * A block no later than `timestamp` and the head. Steps back from the head by the elapsed time
 * over Gnosis' 5-second slot, which lands early enough there in one read because blocks are at
 * least a slot apart; where blocks come faster (a local fork) the step doubles until it does.
 */
export async function blockAtOrBefore(
  read: (number?: bigint) => Promise<BlockStamp>,
  timestamp: bigint,
  slotSeconds = 5n,
): Promise<{ start: bigint; head: bigint }> {
  const head = await read();
  if (head.timestamp <= timestamp) return { start: head.number, head: head.number };
  let step = (head.timestamp - timestamp) / slotSeconds + 1n;
  while (step < head.number) {
    const block = await read(head.number - step);
    if (block.timestamp <= timestamp) return { start: block.number, head: head.number };
    step *= 2n;
  }
  return { start: 0n, head: head.number };
}

export interface PartyEvidence {
  /** The `_evidence` string exactly as filed. */
  uri: string;
  party: Address;
  blockNumber: bigint;
  logIndex: number;
  transactionHash: Hex;
  timestamp?: bigint;
}

export function entryId(entry: PartyEvidence): string {
  return `${entry.transactionHash}:${entry.logIndex}`;
}

export interface EvidenceScan {
  entries: PartyEvidence[];
  range: BlockRange;
  /** The request is older than MAX_SCAN_BLOCKS; blocks after `range.to` were not read. */
  truncated: boolean;
}

export type EvidenceClient = Pick<PublicClient, "getBlock" | "getContractEvents">;

/**
 * The registry's Evidence events for one request: from a block no later than the request's
 * submission, when its group's first evidence can appear, to the head.
 */
export async function findPartyEvidence(
  client: EvidenceClient,
  query: { registry: Address; arbitrator: Address; groupID: bigint; submittedAt: bigint },
  options: { onProgress?: (scannedTo: bigint, range: BlockRange) => void; signal?: AbortSignal } = {},
): Promise<EvidenceScan> {
  const readBlock = async (number?: bigint): Promise<BlockStamp> => {
    const block = number === undefined ? await client.getBlock() : await client.getBlock({ blockNumber: number });
    return { number: block.number, timestamp: block.timestamp };
  };
  const { start, head } = await blockAtOrBefore(readBlock, query.submittedAt);
  const truncated = head - start > MAX_SCAN_BLOCKS;
  const range = { from: start, to: truncated ? start + MAX_SCAN_BLOCKS : head };
  const logs = await readInChunks(
    range,
    (fromBlock, toBlock) =>
      client.getContractEvents({
        address: query.registry,
        abi: generalizedTcrAbi,
        eventName: "Evidence",
        args: { _arbitrator: query.arbitrator, _evidenceGroupID: query.groupID },
        fromBlock,
        toBlock,
        strict: true,
      }),
    { signal: options.signal, onProgress: (scannedTo) => options.onProgress?.(scannedTo, range) },
  );
  const entries: PartyEvidence[] = logs
    .map((log) => ({
      uri: log.args._evidence,
      party: log.args._party,
      blockNumber: log.blockNumber,
      logIndex: log.logIndex,
      transactionHash: log.transactionHash,
    }))
    .sort((a, b) => (a.blockNumber === b.blockNumber ? a.logIndex - b.logIndex : a.blockNumber < b.blockNumber ? -1 : 1));
  const stamps = new Map<bigint, bigint>();
  for (const blockNumber of new Set(entries.slice(0, 100).map((entry) => entry.blockNumber))) {
    stamps.set(blockNumber, (await readBlock(blockNumber)).timestamp);
  }
  for (const entry of entries) entry.timestamp = stamps.get(entry.blockNumber);
  return { entries, range, truncated };
}

const PATH_SEGMENT = /^[A-Za-z0-9\-._~!$&'()*+,;=:@%]+$/;

/**
 * `cid/segment/…` for an IPFS URI (`/ipfs/…` or `ipfs://…`) with a valid CID and plain path
 * segments, or null. Anything else is never fetched.
 */
export function ipfsPath(uri: string): string | null {
  const match = /^(?:\/ipfs\/|ipfs:\/\/(?:ipfs\/)?)([^/?#]+)((?:\/[^/?#]*)*)$/.exec(uri.trim());
  if (!match) return null;
  try {
    CID.parse(match[1]);
  } catch {
    return null;
  }
  const rest = match[2].replace(/\/$/, "");
  const segments = rest === "" ? [] : rest.slice(1).split("/");
  if (segments.some((segment) => segment === "." || segment === ".." || !PATH_SEGMENT.test(segment))) return null;
  return [match[1], ...segments].join("/");
}

export function gatewayUrl(gateway: string, path: string): string {
  return `${gateway.replace(/\/+$/, "")}/ipfs/${path}`;
}

/** Where a file reference may link: IPFS through the gateway, or a plain http(s) URL; nothing else. */
export function evidenceLink(uri: string, gateway: string): string | null {
  const path = ipfsPath(uri);
  if (path) return gatewayUrl(gateway, path);
  try {
    const url = new URL(uri.trim());
    return url.protocol === "https:" || url.protocol === "http:" ? url.href : null;
  } catch {
    return null;
  }
}

export const MAX_EVIDENCE_JSON_BYTES = 64 * 1024;
const MAX_NAME = 300;
const MAX_DESCRIPTION = 20_000;

export interface EvidenceJson {
  name?: string;
  description?: string;
  fileURI?: string;
}

/** The ERC-1497 evidence fields of a JSON object, as text; null when it is not a JSON object. */
export function parseEvidenceJson(bytes: Uint8Array): EvidenceJson | null {
  let value: unknown;
  try {
    value = JSON.parse(new TextDecoder().decode(bytes));
  } catch {
    return null;
  }
  if (!value || typeof value !== "object" || Array.isArray(value)) return null;
  const record = value as Record<string, unknown>;
  const text = (key: string, max: number) => {
    const field = record[key];
    if (typeof field !== "string") return undefined;
    return field.length > max ? `${field.slice(0, max)}… (cut at ${max} characters)` : field;
  };
  return {
    name: text("name", MAX_NAME),
    description: text("description", MAX_DESCRIPTION),
    fileURI: typeof record.fileURI === "string" ? record.fileURI : undefined,
  };
}

export type EvidenceContent =
  | ({ kind: "json"; gateway: string } & EvidenceJson)
  | { kind: "file"; gateway: string; contentType: string }
  | { kind: "unreachable"; detail: string }
  | { kind: "text" };

/**
 * Read one evidence URI through the gateways in order, like the court does: an IPFS URI that
 * holds JSON gives its fields, any other content is a file to link. Other strings are never
 * fetched (an http URL would tell its server who opened the display); they are shown as text.
 */
export async function readEvidence(uri: string, gateways: string[], signal?: AbortSignal, timeoutMs = 15_000): Promise<EvidenceContent> {
  const path = ipfsPath(uri);
  if (!path) return { kind: "text" };
  const failures: string[] = [];
  for (const gateway of gateways) {
    const controller = new AbortController();
    const abort = () => controller.abort();
    signal?.addEventListener("abort", abort);
    const timer = setTimeout(abort, timeoutMs);
    try {
      const response = await fetch(gatewayUrl(gateway, path), { signal: controller.signal });
      if (!response.ok) {
        failures.push(`${gateway}: HTTP ${response.status}`);
        continue;
      }
      const contentType = response.headers.get("content-type") ?? "";
      if (!/json|^text\/plain/i.test(contentType)) {
        await response.body?.cancel();
        return { kind: "file", gateway, contentType };
      }
      const bytes = await readCapped(response, MAX_EVIDENCE_JSON_BYTES);
      const fields = bytes ? parseEvidenceJson(bytes) : null;
      return fields ? { kind: "json", gateway, ...fields } : { kind: "file", gateway, contentType };
    } catch (caught) {
      if (signal?.aborted) throw caught;
      failures.push(`${gateway}: ${controller.signal.aborted ? "timed out" : caught instanceof Error ? caught.message : String(caught)}`);
    } finally {
      clearTimeout(timer);
      signal?.removeEventListener("abort", abort);
    }
  }
  return { kind: "unreachable", detail: failures.join("; ") };
}
