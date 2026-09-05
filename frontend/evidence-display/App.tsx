import { useEffect, useMemo, useState } from "react";
import Markdown from "react-markdown";
import remarkGfm from "remark-gfm";
import { createPublicClient, http, type Chain } from "viem";
import { generalizedTcrAbi } from "../src/abi/GeneralizedTCR";
import { gnosis, localFork } from "../src/config/chains";
import { decodeItem } from "../src/lib/encoder";
import { frontmatterBlock, parseFrontmatter } from "../src/lib/frontmatter";
import { getItemValidationError, isCanonicalTreeCid, originHref } from "../src/lib/schema";
import {
  DEFAULT_GATEWAYS,
  DEFAULT_LIMITS,
  fetchTreeCar,
  INTEGRITY_CODES,
  isProbablyText,
  verifyTreeCar,
  type TreeIssue,
  type VerifiedTree,
} from "../src/lib/tree";
import { ItemStatus, type ItemFields } from "../src/types";
import { readParams, type DisplayParams } from "./params";

const ZERO_ITEM = `0x${"0".repeat(64)}` as const;
const MAX_TEXT_VIEW = 512 * 1024;

interface LoadedItem {
  registry: `0x${string}`;
  itemID: `0x${string}`;
  status: number;
  fields?: ItemFields;
  descriptorError?: string;
  request?: {
    index: number;
    disputed: boolean;
    disputeID: bigint;
    resolved: boolean;
    requester: `0x${string}`;
    challenger: `0x${string}`;
    ruling: number;
  };
}

type TreeState =
  | { phase: "fetching" }
  | { phase: "verified"; tree: VerifiedTree; gateway: string }
  | { phase: "failed"; error: string };

/** The outcome for one CID; anything else, or a different CID, means the fetch is still running. */
type TreeResult = { cid: string } & (Exclude<TreeState, { phase: "fetching" }>);

function chainFor(id: number | undefined): Chain {
  if (id === localFork.id) return localFork;
  return gnosis;
}

function gatewaysFromEnv(): string[] {
  const configured = (import.meta.env.VITE_IPFS_GATEWAYS as string | undefined)?.split(",").map((g) => g.trim()).filter(Boolean);
  return configured && configured.length > 0 ? configured : DEFAULT_GATEWAYS;
}

function formatBytes(n: number): string {
  if (n < 1024) return `${n} B`;
  if (n < 1024 * 1024) return `${(n / 1024).toFixed(1)} KiB`;
  return `${(n / (1024 * 1024)).toFixed(2)} MiB`;
}

function shortAddress(address: string): string {
  return `${address.slice(0, 6)}…${address.slice(-4)}`;
}

async function loadItem(params: DisplayParams): Promise<LoadedItem> {
  const registry = params.arbitrableContractAddress;
  if (!registry) throw new Error("No registry address was passed to the display");
  const chain = chainFor(params.arbitrableChainID);
  const rpc = params.arbitrableJsonRpcUrl || (import.meta.env.VITE_RPC_URL as string | undefined) || undefined;
  const client = createPublicClient({ chain, transport: http(rpc) });
  const contract = { address: registry, abi: generalizedTcrAbi } as const;

  let itemID = params.itemID;
  if (!itemID) {
    if (!params.disputeID || !params.arbitratorContractAddress) {
      throw new Error("The display needs either an item ID or a dispute ID with its arbitrator");
    }
    itemID = await client.readContract({
      ...contract,
      functionName: "arbitratorDisputeIDToItem",
      args: [params.arbitratorContractAddress, BigInt(params.disputeID)],
    });
    if (itemID === ZERO_ITEM) throw new Error(`Dispute ${params.disputeID} on ${shortAddress(params.arbitratorContractAddress)} maps to no item in this registry`);
  }

  const [data, status, numberOfRequests] = await client.readContract({ ...contract, functionName: "getItemInfo", args: [itemID] });
  const loaded: LoadedItem = { registry, itemID, status: Number(status) };
  try {
    if (data && data !== "0x") {
      loaded.fields = decodeItem(data);
      loaded.descriptorError = getItemValidationError(loaded.fields) ?? undefined;
    } else {
      loaded.descriptorError = "The registry holds no descriptor for this item";
    }
  } catch (caught) {
    loaded.descriptorError = caught instanceof Error ? caught.message : "Descriptor could not be decoded";
  }
  const count = Number(numberOfRequests);
  if (count > 0) {
    const index = count - 1;
    const info = await client.readContract({ ...contract, functionName: "getRequestInfo", args: [itemID, BigInt(index)] });
    const [disputed, disputeID, , resolved, parties, , ruling] = info;
    loaded.request = { index, disputed, disputeID, resolved, requester: parties[1], challenger: parties[2], ruling: Number(ruling) };
  }
  return loaded;
}

type Verdict = "pass" | "fail" | "warn" | "info";
interface CheckRow {
  label: string;
  verdict: Verdict;
  detail: string;
}

function checklist(item: LoadedItem, treeState: TreeState): CheckRow[] {
  const rows: CheckRow[] = [];
  const fields = item.fields;
  rows.push(
    item.descriptorError
      ? { label: "Descriptor", verdict: "fail", detail: item.descriptorError }
      : { label: "Descriptor", verdict: "pass", detail: "Canonical RLP of six UTF-8 columns; every column passes the policy's form rules" },
  );
  if (!fields) return rows;
  rows.push(
    isCanonicalTreeCid(fields.treeCid)
      ? { label: "Tree CID form", verdict: "pass", detail: "CIDv1, DAG-PB, 32-byte SHA-256, canonical base32" }
      : { label: "Tree CID form", verdict: "fail", detail: "Not the policy's CID form" },
  );
  if (treeState.phase !== "verified") {
    rows.push({
      label: "Skill tree",
      verdict: treeState.phase === "failed" ? "fail" : "info",
      detail: treeState.phase === "failed" ? treeState.error : "Fetching the complete tree as a CAR and verifying every block…",
    });
    return rows;
  }
  const { tree } = treeState;
  const integrity = tree.issues.filter((issue) => INTEGRITY_CODES.has(issue.code));
  const profile = tree.issues.filter((issue) => !INTEGRITY_CODES.has(issue.code) && issue.code !== "TREE_SIZE" && issue.code !== "CAR_ROOTS");
  const size = tree.issues.find((issue) => issue.code === "TREE_SIZE");
  rows.push(
    integrity.length === 0
      ? { label: "Complete DAG, every block hash verified", verdict: "pass", detail: `${tree.stats.reachableBlocks} blocks reachable from the root, all SHA-256 verified, DAG-PB and raw only` }
      : { label: "Complete DAG, every block hash verified", verdict: "fail", detail: integrity.map((issue) => issue.message).join("; ") },
  );
  rows.push(
    profile.length === 0
      ? { label: "No symlinks; CIDv1 links; inside the installer profile", verdict: "pass", detail: "Plain directories, raw leaves, single-level chunked files, sorted unique names, every link CIDv1" }
      : { label: "No symlinks; inside the installer profile", verdict: profile.some((issue) => issue.code === "SYMLINK" || issue.code === "CID_VERSION") ? "fail" : "warn", detail: profile.map((issue) => issue.message).join("; ") },
  );
  rows.push(
    size
      ? { label: "Total size within 2 MiB (criterion 9)", verdict: "fail", detail: size.message }
      : { label: "Total size within 2 MiB (criterion 9)", verdict: "pass", detail: `${formatBytes(tree.stats.treeBytes)} across ${tree.entries.filter((e) => e.kind === "file").length} files` },
  );
  if (tree.has("SKILL.md")) {
    const bytes = tree.read("SKILL.md");
    try {
      const fm = parseFrontmatter(new TextDecoder("utf-8", { fatal: true }).decode(bytes));
      const nameOk = fm.name === fields.name;
      const descOk = fm.description === fields.description;
      rows.push(
        nameOk && descOk
          ? { label: "Frontmatter binds the descriptor (criterion 3)", verdict: "pass", detail: "SKILL.md name and description are byte-identical to the descriptor" }
          : {
              label: "Frontmatter binds the descriptor (criterion 3)",
              verdict: "fail",
              detail: [!nameOk && `name differs: frontmatter ${JSON.stringify(fm.name)}`, !descOk && "description differs from the descriptor"].filter(Boolean).join("; "),
            },
      );
    } catch (caught) {
      rows.push({ label: "Frontmatter binds the descriptor (criterion 3)", verdict: "fail", detail: caught instanceof Error ? caught.message : String(caught) });
    }
  } else {
    rows.push({ label: "Frontmatter binds the descriptor (criterion 3)", verdict: "fail", detail: "The tree has no SKILL.md at its root" });
  }
  if (tree.stats.hasModes || tree.stats.hasMtimes) {
    rows.push({ label: "UnixFS metadata", verdict: "info", detail: "The tree carries mode or mtime metadata; the installer ignores it and executable bits are not semantic under the policy" });
  }
  if (tree.stats.unreachableBlocks > 0) {
    rows.push({ label: "Extra blocks", verdict: "info", detail: `${tree.stats.unreachableBlocks} blocks in the CAR are not part of this tree and were ignored` });
  }
  rows.push(
    fields.origin === ""
      ? { label: "Origin (criterion 6)", verdict: "info", detail: "No origin claimed" }
      : { label: "Origin (criterion 6)", verdict: "info", detail: "An origin is claimed. The policy requires it to bind this exact semantic tree; check the linked commit or publisher against the file list below" },
  );
  return rows;
}

const verdictStyle: Record<Verdict, string> = {
  pass: "bg-emerald-100 text-emerald-900",
  fail: "bg-red-100 text-red-900",
  warn: "bg-amber-100 text-amber-900",
  info: "bg-slate-100 text-slate-800",
};
const verdictWord: Record<Verdict, string> = { pass: "pass", fail: "fail", warn: "check", info: "note" };

function statusLabel(status: number): string {
  switch (status) {
    case ItemStatus.Absent:
      return "absent";
    case ItemStatus.Registered:
      return "registered";
    case ItemStatus.RegistrationRequested:
      return "registration requested";
    case ItemStatus.ClearingRequested:
      return "removal requested";
    default:
      return `status ${status}`;
  }
}

export function App() {
  const params = useMemo(() => {
    try {
      return { params: readParams(window.location.search) };
    } catch (caught) {
      return { error: `Could not read the display parameters: ${caught instanceof Error ? caught.message : String(caught)}` };
    }
  }, []);
  const [item, setItem] = useState<LoadedItem | null>(null);
  const [itemError, setItemError] = useState<string | null>(params.error ?? null);
  const [treeResult, setTreeResult] = useState<TreeResult | null>(null);
  const [selected, setSelected] = useState<string>("SKILL.md");

  useEffect(() => {
    if (!params.params) return;
    let cancelled = false;
    loadItem(params.params)
      .then((loaded) => {
        if (!cancelled) setItem(loaded);
      })
      .catch((caught) => {
        if (!cancelled) setItemError(caught instanceof Error ? caught.message : String(caught));
      });
    return () => {
      cancelled = true;
    };
  }, [params]);

  useEffect(() => {
    const cid = item?.fields?.treeCid;
    if (!cid || !isCanonicalTreeCid(cid)) return;
    const controller = new AbortController();
    fetchTreeCar(cid, gatewaysFromEnv(), DEFAULT_LIMITS.maxCarBytes, controller.signal)
      .then(async ({ bytes, gateway }) => {
        const tree = await verifyTreeCar(bytes, cid);
        if (!controller.signal.aborted) setTreeResult({ cid, phase: "verified", tree, gateway });
      })
      .catch((caught) => {
        if (!controller.signal.aborted) setTreeResult({ cid, phase: "failed", error: caught instanceof Error ? caught.message : String(caught) });
      });
    return () => controller.abort();
  }, [item]);

  if (itemError) {
    return (
      <main className="mx-auto max-w-3xl p-6 text-sm text-gray-800">
        <h1 className="text-lg font-semibold">Intendancy skill review</h1>
        <p className="mt-3 rounded bg-red-50 p-3 text-red-900">{itemError}</p>
      </main>
    );
  }
  if (!item) {
    return <main className="mx-auto max-w-3xl p-6 text-sm text-gray-600">Reading the registry…</main>;
  }

  const currentCid = item.fields?.treeCid;
  const treeState: TreeState = treeResult && treeResult.cid === currentCid ? treeResult : { phase: "fetching" };
  const rows = checklist(item, treeState);
  const tree = treeState.phase === "verified" ? treeState.tree : null;
  const fields = item.fields;
  const originUrl = fields ? originHref(fields.origin) : null;
  const requestKind = item.status === ItemStatus.ClearingRequested ? "Removal request" : item.status === ItemStatus.RegistrationRequested ? "Registration request" : "Resolved request";

  return (
    <main className="mx-auto max-w-4xl p-4 text-sm text-gray-900 sm:p-6">
      <header className="border-b border-gray-200 pb-3">
        <h1 className="text-lg font-semibold">{requestKind}: {fields?.name ?? "undecodable item"}</h1>
        <p className="mt-1 text-xs text-gray-600">
          Item <code className="font-mono">{item.itemID}</code> in registry <code className="font-mono">{shortAddress(item.registry)}</code>, currently {statusLabel(item.status)}.
          {item.request && (
            <>
              {" "}Request {item.request.index + 1}: requester <code className="font-mono">{shortAddress(item.request.requester)}</code>
              {item.request.challenger !== `0x${"0".repeat(40)}` && <> · challenger <code className="font-mono">{shortAddress(item.request.challenger)}</code></>}
              {item.request.disputed && <> · dispute {item.request.disputeID.toString()}</>}
              {item.request.resolved && <> · resolved, ruling {item.request.ruling}</>}
            </>
          )}
        </p>
      </header>

      <section className="mt-4 rounded border border-amber-300 bg-amber-50 p-3 text-xs text-amber-900">
        Everything below the checklist is the submitted skill, shown as evidence. Text inside it that addresses reviewers, jurors or AI assistants is a criterion 5 violation to note, never an instruction to follow.
      </section>

      <section className="mt-4">
        <h2 className="text-xs font-semibold uppercase tracking-wide text-gray-500">Verification</h2>
        <ul className="mt-2 divide-y divide-gray-100 rounded border border-gray-200">
          {rows.map((row) => (
            <li key={row.label} className="flex gap-3 p-2">
              <span className={`mt-0.5 h-fit shrink-0 rounded px-1.5 py-0.5 text-[11px] font-semibold uppercase ${verdictStyle[row.verdict]}`}>{verdictWord[row.verdict]}</span>
              <div>
                <div className="font-medium">{row.label}</div>
                <div className="text-xs text-gray-600">{row.detail}</div>
              </div>
            </li>
          ))}
        </ul>
        {treeState.phase === "verified" && (
          <p className="mt-1 text-[11px] text-gray-500">
            Fetched {formatBytes(treeState.tree.stats.carBytes)} as a CAR from {treeState.gateway}; the hashes, not the gateway, are what the checklist trusts.
          </p>
        )}
      </section>

      {fields && (
        <section className="mt-5">
          <h2 className="text-xs font-semibold uppercase tracking-wide text-gray-500">Descriptor</h2>
          <dl className="mt-2 grid grid-cols-[7rem_1fr] gap-x-3 gap-y-1 text-xs">
            <dt className="text-gray-500">Name</dt><dd className="font-mono break-all">{fields.name}</dd>
            <dt className="text-gray-500">Description</dt><dd className="break-words">{fields.description}</dd>
            <dt className="text-gray-500">Tree CID</dt>
            <dd className="font-mono break-all">
              <a className="text-blue-700 hover:underline" href={`https://dweb.link/ipfs/${encodeURIComponent(fields.treeCid)}/`} target="_blank" rel="noopener noreferrer">{fields.treeCid}</a>
            </dd>
            <dt className="text-gray-500">Runtimes</dt><dd className="font-mono">{fields.runtimes}</dd>
            <dt className="text-gray-500">Origin</dt>
            <dd className="font-mono break-all">
              {fields.origin === "" ? <span className="text-gray-400">none</span> : originUrl ? <a className="text-blue-700 hover:underline" href={originUrl} target="_blank" rel="noopener noreferrer">{fields.origin}</a> : fields.origin}
            </dd>
            <dt className="text-gray-500">Reserved</dt><dd className="font-mono">{fields.reserved === "" ? <span className="text-gray-400">empty</span> : <span className="text-red-700">{JSON.stringify(fields.reserved)}</span>}</dd>
          </dl>
        </section>
      )}

      {tree && (
        <section className="mt-5 grid gap-4 md:grid-cols-[16rem_1fr]">
          <div>
            <h2 className="text-xs font-semibold uppercase tracking-wide text-gray-500">Files</h2>
            <ul className="mt-2 max-h-[32rem] overflow-auto rounded border border-gray-200 text-xs">
              {tree.entries.filter((entry) => entry.kind === "file").map((entry) => (
                <li key={entry.path}>
                  <button
                    type="button"
                    onClick={() => setSelected(entry.path)}
                    className={`flex w-full items-baseline justify-between gap-2 px-2 py-1 text-left font-mono hover:bg-gray-50 focus:outline-none focus-visible:ring-2 focus-visible:ring-blue-500 ${selected === entry.path ? "bg-blue-50" : ""}`}
                  >
                    <span className="break-all">{entry.path}</span>
                    <span className="shrink-0 text-gray-500">{formatBytes(entry.size)}</span>
                  </button>
                </li>
              ))}
            </ul>
          </div>
          <FileView tree={tree} path={selected} />
        </section>
      )}
    </main>
  );
}

function FileView({ tree, path }: { tree: VerifiedTree; path: string }) {
  const content = useMemo(() => {
    if (!tree.has(path)) return { kind: "missing" as const };
    const bytes = tree.read(path);
    if (bytes.length > MAX_TEXT_VIEW || !isProbablyText(bytes)) return { kind: "binary" as const, size: bytes.length };
    const text = new TextDecoder().decode(bytes);
    if (!path.toLowerCase().endsWith(".md")) return { kind: "text" as const, text };
    // Show a SKILL.md frontmatter block verbatim, then render the body; a
    // renderer would otherwise turn the block into a rule and a run-on line.
    let frontmatter: string | null = null;
    let body = text;
    if (path === "SKILL.md") {
      try {
        frontmatter = frontmatterBlock(text);
        const afterOpening = text.slice(text.indexOf("\n") + 1);
        const closing = afterOpening.indexOf("\n---");
        body = closing >= 0 ? afterOpening.slice(afterOpening.indexOf("\n", closing + 1) + 1) : "";
      } catch {
        frontmatter = null;
      }
    }
    return { kind: "markdown" as const, text: body, frontmatter };
  }, [tree, path]);

  return (
    <div className="min-w-0">
      <h2 className="text-xs font-semibold uppercase tracking-wide text-gray-500">{path}</h2>
      <div className="mt-2 max-h-[32rem] overflow-auto rounded border border-gray-200 p-3">
        {content.kind === "missing" && <p className="text-xs text-gray-500">This tree has no file at {path}.</p>}
        {content.kind === "binary" && <p className="text-xs text-gray-500">Binary or oversized content, {formatBytes(content.size)}; not rendered.</p>}
        {content.kind === "text" && <pre className="whitespace-pre-wrap break-words font-mono text-xs">{content.text}</pre>}
        {content.kind === "markdown" && (
          <div>
            {content.frontmatter !== null && (
              <div className="mb-3">
                <div className="text-[10px] font-semibold uppercase tracking-wide text-gray-500">frontmatter, as submitted</div>
                <pre className="mt-1 whitespace-pre-wrap break-words rounded bg-gray-50 p-2 font-mono text-xs">{content.frontmatter}</pre>
              </div>
            )}
            <div className="md">
              <Markdown remarkPlugins={[remarkGfm]}>{content.text}</Markdown>
            </div>
          </div>
        )}
      </div>
    </div>
  );
}

export type { TreeIssue };
