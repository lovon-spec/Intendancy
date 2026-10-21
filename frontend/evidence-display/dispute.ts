import {
  BaseError,
  ContractFunctionRevertedError,
  ContractFunctionZeroDataError,
  isAddressEqual,
  type Address,
  type Hex,
  type PublicClient,
} from "viem";
import { generalizedTcrAbi } from "../src/abi/GeneralizedTCR";
import { intendmentArbitratorAbi } from "../src/abi/IntendmentArbitrator";
import type { DisplayParams } from "./params";

export const ZERO_ITEM = `0x${"0".repeat(64)}` as const;

/** The reads the mapping needs: a viem client, or a fake in tests. */
export type ContractReader = Pick<PublicClient, "readContract">;

/** A Kleros dispute that reached the registry through an Intendment wrapper. */
export interface WrapperRoute {
  wrapper: Address;
  /** The court the wrapper forwards to; the same arbitrator that passed the dispute. */
  court: Address;
  /** The court's dispute ID. */
  remoteID: bigint;
  /** The wrapper's case ID, which is the dispute ID the registry holds. */
  localID: bigint;
}

export interface DisputeTarget {
  registry: Address;
  itemID: Hex;
  /** The disputed request, when the route pins it (the wrapper's binding). */
  requestIndex?: number;
  /** The dispute as the registry recorded it: the arbitrator it called and the ID that arbitrator returned. */
  registryDispute?: { arbitrator: Address; disputeID: bigint };
  viaWrapper?: WrapperRoute;
}

export function shortAddress(address: string): string {
  return `${address.slice(0, 6)}…${address.slice(-4)}`;
}

/** A call that reverted or returned nothing: the contract lacks the function, or refused the input. */
function isRevert(error: unknown): boolean {
  return (
    error instanceof BaseError &&
    Boolean(error.walk((cause) => cause instanceof ContractFunctionRevertedError || cause instanceof ContractFunctionZeroDataError))
  );
}

function parseDisputeID(value: string): bigint {
  if (!/^\d{1,78}$/.test(value)) throw new Error(`The dispute ID ${JSON.stringify(value)} is not a number`);
  return BigInt(value);
}

/** The wrapper's registry, or null when the arbitrable is not a wrapper (a registry has no HOST()). */
async function wrapperHost(client: ContractReader, arbitrable: Address): Promise<Address | null> {
  try {
    return await client.readContract({ address: arbitrable, abi: intendmentArbitratorAbi, functionName: "HOST" });
  } catch (error) {
    if (isRevert(error)) return null;
    throw error;
  }
}

/**
 * Map the court's parameters to a registry item. A registry that is its own arbitrable maps the
 * dispute directly. When the arbitrable is an Intendment wrapper, the court's (remote) dispute ID
 * maps to the wrapper's local case, and the registry knows that case as the dispute.
 */
export async function resolveDispute(client: ContractReader, params: DisplayParams): Promise<DisputeTarget> {
  const arbitrable = params.arbitrableContractAddress;
  if (!arbitrable) throw new Error("No registry address was passed to the display");
  if (params.itemID) return { registry: arbitrable, itemID: params.itemID };
  if (!params.disputeID || !params.arbitratorContractAddress) {
    throw new Error("The display needs either an item ID or a dispute ID with its arbitrator");
  }
  const arbitrator = params.arbitratorContractAddress;
  const disputeID = parseDisputeID(params.disputeID);

  const host = await wrapperHost(client, arbitrable);
  if (!host) {
    const itemID = await client.readContract({
      address: arbitrable,
      abi: generalizedTcrAbi,
      functionName: "arbitratorDisputeIDToItem",
      args: [arbitrator, disputeID],
    });
    if (itemID === ZERO_ITEM) throw new Error(`Dispute ${disputeID} on ${shortAddress(arbitrator)} maps to no item in this registry`);
    return { registry: arbitrable, itemID, registryDispute: { arbitrator, disputeID } };
  }

  const wrapper = arbitrable;
  const court = await client.readContract({ address: wrapper, abi: intendmentArbitratorAbi, functionName: "K" });
  if (!isAddressEqual(court, arbitrator)) {
    throw new Error(`The wrapper ${shortAddress(wrapper)} forwards to ${shortAddress(court)}, not to the arbitrator ${shortAddress(arbitrator)} that passed dispute ${disputeID}`);
  }
  let localID: bigint;
  try {
    localID = await client.readContract({ address: wrapper, abi: intendmentArbitratorAbi, functionName: "localDispute", args: [disputeID] });
  } catch (error) {
    if (isRevert(error)) throw new Error(`Dispute ${disputeID} on ${shortAddress(court)} was not forwarded by the wrapper ${shortAddress(wrapper)}`);
    throw error;
  }
  const itemID = await client.readContract({
    address: host,
    abi: generalizedTcrAbi,
    functionName: "arbitratorDisputeIDToItem",
    args: [wrapper, localID],
  });
  if (itemID === ZERO_ITEM) throw new Error(`Wrapper case ${localID} maps to no item in the registry ${shortAddress(host)}`);
  // The case was bound to the registry's request when it was forwarded; it must agree with both IDs.
  const record = await client.readContract({ address: wrapper, abi: intendmentArbitratorAbi, functionName: "caseOf", args: [localID] });
  if (record.kDisputeID !== disputeID || record.binding.item.toLowerCase() !== itemID.toLowerCase()) {
    throw new Error(`The wrapper's case ${localID} records dispute ${record.kDisputeID} and item ${record.binding.item}, which do not match dispute ${disputeID} and item ${itemID}`);
  }
  return {
    registry: host,
    itemID,
    requestIndex: Number(record.binding.requestIndex),
    registryDispute: { arbitrator: wrapper, disputeID: localID },
    viaWrapper: { wrapper, court, remoteID: disputeID, localID },
  };
}

/** The fields of getRequestInfo the display reads. */
export interface RequestFields {
  disputed: boolean;
  disputeID: bigint;
  submissionTime: bigint;
  resolved: boolean;
  requester: Address;
  challenger: Address;
  ruling: number;
  arbitrator: Address;
}

/**
 * The request a dispute belongs to: the hinted one (the wrapper's binding) or else the latest,
 * as long as it recorded this arbitrator and dispute ID. After the dispute the item may have a
 * newer request, so otherwise walk back to the one that did. Without a dispute, or if no request
 * matches, the first candidate is the answer.
 */
export async function findRequest(
  count: number,
  read: (index: number) => Promise<RequestFields>,
  dispute?: { arbitrator: Address; disputeID: bigint },
  hint?: number,
): Promise<{ index: number; fields: RequestFields }> {
  const first = hint !== undefined && hint >= 0 && hint < count ? hint : count - 1;
  const candidate = { index: first, fields: await read(first) };
  if (!dispute) return candidate;
  const matches = (fields: RequestFields) =>
    fields.disputed && fields.disputeID === dispute.disputeID && isAddressEqual(fields.arbitrator, dispute.arbitrator);
  if (matches(candidate.fields)) return candidate;
  for (let index = count - 1; index >= 0; index--) {
    if (index === first) continue;
    const fields = await read(index);
    if (matches(fields)) return { index, fields };
  }
  return candidate;
}
