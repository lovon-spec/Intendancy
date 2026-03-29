import { useReadContract, useReadContracts } from "wagmi";
import { generalizedTcrAbi } from "../abi/GeneralizedTCR";
import { REGISTRY_ADDRESS } from "../config/registry";
import { decodeItem } from "../lib/encoder";
import { getDisplayStatus } from "../lib/status";
import { ItemStatus, type RegistryItem } from "../types";

const contract = { address: REGISTRY_ADDRESS, abi: generalizedTcrAbi } as const;

export function useRegistryItems(): {
  data: RegistryItem[];
  isLoading: boolean;
  error: Error | null;
} {
  // Step 1: get item count
  const { data: itemCount } = useReadContract({
    ...contract,
    functionName: "itemCount",
  });

  const count = Number(itemCount ?? 0);

  // Step 2: get all item IDs
  const idContracts = Array.from({ length: count }, (_, i) => ({
    ...contract,
    functionName: "itemList" as const,
    args: [BigInt(i)] as const,
  }));

  const { data: idResults } = useReadContracts({ contracts: idContracts, query: { enabled: count > 0 } });

  const itemIDs = (idResults ?? [])
    .filter((r) => r.status === "success")
    .map((r) => r.result as `0x${string}`);

  // Step 3: get item info for each
  const infoContracts = itemIDs.map((id) => ({
    ...contract,
    functionName: "getItemInfo" as const,
    args: [id] as const,
  }));

  const { data: infoResults, isLoading, error } = useReadContracts({
    contracts: infoContracts,
    query: { enabled: itemIDs.length > 0 },
  });

  // Step 4: fetch request info where needed:
  // - Pending/Clearing items: latest request (for disputed flag + requester)
  // - Registered items: first request (for submissionTime → long-standing badge)
  const extraRequestIndices: number[] = [];
  const extraRequestContracts: Array<{
    address: `0x${string}`;
    abi: typeof generalizedTcrAbi;
    functionName: "getRequestInfo";
    args: readonly [`0x${string}`, bigint];
  }> = [];

  if (infoResults) {
    infoResults.forEach((info, i) => {
      if (info.status !== "success") return;
      const [, status, numRequests] = info.result as [string, number, bigint];
      if (numRequests === 0n) return;

      if (status === ItemStatus.RegistrationRequested || status === ItemStatus.ClearingRequested) {
        // Latest request for pending items
        extraRequestIndices.push(i);
        extraRequestContracts.push({
          ...contract,
          functionName: "getRequestInfo",
          args: [itemIDs[i], numRequests - 1n],
        });
      } else if (status === ItemStatus.Registered) {
        // First request for registered items (registration time)
        extraRequestIndices.push(i);
        extraRequestContracts.push({
          ...contract,
          functionName: "getRequestInfo",
          args: [itemIDs[i], 0n],
        });
      }
    });
  }

  const { data: extraResults } = useReadContracts({
    contracts: extraRequestContracts,
    query: { enabled: extraRequestContracts.length > 0 },
  });

  // Build lookup: item index → extra info
  const extraInfoMap = new Map<number, { disputed: boolean; requester: `0x${string}`; submissionTime: bigint }>();
  if (extraResults) {
    extraResults.forEach((r, i) => {
      if (r.status !== "success") return;
      const result = r.result as readonly [boolean, bigint, bigint, boolean, readonly [`0x${string}`, `0x${string}`, `0x${string}`], bigint, number, string, string, bigint];
      extraInfoMap.set(extraRequestIndices[i], {
        disputed: result[0],
        requester: result[4][1],
        submissionTime: result[2],
      });
    });
  }

  // Assemble items
  const items: RegistryItem[] = [];
  if (infoResults) {
    infoResults.forEach((info, i) => {
      if (info.status !== "success") return;
      const [rawData, status, numRequests] = info.result as [`0x${string}`, number, bigint];
      try {
        const fields = decodeItem(rawData);
        const extra = extraInfoMap.get(i);
        const displayStatus = getDisplayStatus(
          status as ItemStatus,
          extra?.disputed ?? false,
          extra?.requester,
          status === ItemStatus.Registered ? extra?.submissionTime : undefined,
        );
        items.push({
          itemID: itemIDs[i],
          fields,
          rawData,
          status: status as ItemStatus,
          numberOfRequests: Number(numRequests),
          displayStatus,
        });
      } catch {
        // Skip items that can't be decoded (different schema)
      }
    });
  }

  return { data: items, isLoading, error: error as Error | null };
}
