import { isAddress } from "viem";

/** What the court's evidence frame passes, plus the plain form our own site and tests use. */
export interface DisplayParams {
  disputeID?: string;
  arbitrableContractAddress?: `0x${string}`;
  arbitratorContractAddress?: `0x${string}`;
  arbitrableChainID?: number;
  arbitrableJsonRpcUrl?: string;
  itemID?: `0x${string}`;
}

const asAddress = (value: unknown): `0x${string}` | undefined =>
  typeof value === "string" && isAddress(value, { strict: false }) ? (value as `0x${string}`) : undefined;
const asString = (value: unknown): string | undefined => (typeof value === "string" && value !== "" ? value : undefined);
const asNumber = (value: unknown): number | undefined => {
  const parsed = typeof value === "number" ? value : typeof value === "string" ? Number(value) : NaN;
  return Number.isSafeInteger(parsed) && parsed > 0 ? parsed : undefined;
};
const asItemID = (value: unknown): `0x${string}` | undefined =>
  typeof value === "string" && /^0x[0-9a-fA-F]{64}$/.test(value) ? (value as `0x${string}`) : undefined;

/**
 * The Kleros court loads the display as `index.html?{"disputeID":…,"arbitrableContractAddress":…,…}`,
 * URL-encoded. Our own site uses `?registry=0x…&item=0x…[&chain=100][&rpc=…]`.
 */
export function readParams(search: string): DisplayParams {
  const raw = search.startsWith("?") ? search.slice(1) : search;
  if (raw === "") return {};
  if (raw.startsWith("{") || raw.startsWith("%7B") || raw.startsWith("%7b")) {
    let text: string;
    try {
      text = decodeURIComponent(raw);
    } catch {
      text = raw;
    }
    if (!text.startsWith("{")) {
      text = raw.replace(/%22/g, '"').replace(/%7B/gi, "{").replace(/%3A/gi, ":").replace(/%2C/gi, ",").replace(/%7D/gi, "}");
    }
    const object = JSON.parse(text) as Record<string, unknown>;
    return {
      disputeID: asString(object.disputeID),
      arbitrableContractAddress: asAddress(object.arbitrableContractAddress),
      arbitratorContractAddress: asAddress(object.arbitratorContractAddress),
      arbitrableChainID: asNumber(object.arbitrableChainID),
      arbitrableJsonRpcUrl: asString(object.arbitrableJsonRpcUrl),
    };
  }
  const query = new URLSearchParams(raw);
  return {
    arbitrableContractAddress: asAddress(query.get("registry")),
    itemID: asItemID(query.get("item")),
    disputeID: asString(query.get("dispute")),
    arbitratorContractAddress: asAddress(query.get("arbitrator")),
    arbitrableChainID: asNumber(query.get("chain")),
    arbitrableJsonRpcUrl: asString(query.get("rpc")),
  };
}
