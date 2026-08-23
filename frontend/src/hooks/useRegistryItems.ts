import { useReadContract, useReadContracts } from "wagmi";
import { generalizedTcrAbi } from "../abi/GeneralizedTCR";
import { REGISTRY_ADDRESS } from "../config/registry";
import { decodeItem } from "../lib/encoder";
import { getItemValidationError } from "../lib/schema";
import { getDisplayStatus, isItemStatus } from "../lib/status";
import { ItemStatus, type RegistryItem } from "../types";

const contract = { address: REGISTRY_ADDRESS, abi: generalizedTcrAbi } as const;

export function useRegistryItems(): {
  data: RegistryItem[];
  isLoading: boolean;
  error: Error | null;
} {
  // Step 1: get item count
  const { data: itemCount, isLoading: countLoading, error: countError } = useReadContract({
    ...contract,
    functionName: "itemCount",
  });

  const countIsSafe = itemCount === undefined || itemCount <= BigInt(Number.MAX_SAFE_INTEGER);
  const count = countIsSafe ? Number(itemCount ?? 0) : 0;

  // Step 2: get all item IDs
  const idContracts = Array.from({ length: count }, (_, i) => ({
    ...contract,
    functionName: "itemList" as const,
    args: [BigInt(i)] as const,
  }));

  const { data: idResults, isLoading: idsLoading, error: idsError } = useReadContracts({
    contracts: idContracts,
    query: { enabled: count > 0 },
  });

  const itemRefs = (idResults ?? []).flatMap((result, index) =>
    result.status === "success"
      ? [{ index, itemID: result.result as `0x${string}` }]
      : [],
  );
  const itemIDs = itemRefs.map(({ itemID }) => itemID);

  // Step 3: get item info for each
  const infoContracts = itemIDs.map((id) => ({
    ...contract,
    functionName: "getItemInfo" as const,
    args: [id] as const,
  }));

  const { data: infoResults, isLoading: infoLoading, error: infoError } = useReadContracts({
    contracts: infoContracts,
    query: { enabled: itemIDs.length > 0 },
  });

  // Step 4: fetch the latest pending request so both registration and removal
  // disputes remain visible without weakening ClearingRequested quarantine.
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
        extraRequestIndices.push(i);
        extraRequestContracts.push({
          ...contract,
          functionName: "getRequestInfo",
          args: [itemIDs[i], numRequests - 1n],
        });
      }
    });
  }

  const { data: extraResults, isLoading: extraLoading, error: extraError } = useReadContracts({
    contracts: extraRequestContracts,
    query: { enabled: extraRequestContracts.length > 0 },
  });

  // Build lookup: item index → extra info
  const extraInfoMap = new Map<number, { disputed: boolean }>();
  if (extraResults) {
    extraResults.forEach((r, i) => {
      if (r.status !== "success") return;
      const result = r.result as readonly [boolean, bigint, bigint, boolean, readonly [`0x${string}`, `0x${string}`, `0x${string}`], bigint, number, string, string, bigint];
      extraInfoMap.set(extraRequestIndices[i], {
        disputed: result[0],
      });
    });
  }

  // Assemble items
  const items: RegistryItem[] = [];
  let invalidStatus: number | null = null;
  if (infoResults) {
    infoResults.forEach((info, i) => {
      if (info.status !== "success") return;
      const [rawData, status, numRequests] = info.result as [`0x${string}`, number, bigint];
      if (!isItemStatus(status)) {
        invalidStatus = status;
        return;
      }
      const extra = extraInfoMap.get(i);
      const displayStatus = getDisplayStatus(
        status,
        extra?.disputed ?? false,
      );
      try {
        const fields = decodeItem(rawData);
        items.push({
          index: itemRefs[i].index,
          itemID: itemRefs[i].itemID,
          fields,
          descriptorError: getItemValidationError(fields) ?? undefined,
          rawData,
          status,
          numberOfRequests: Number(numRequests),
          requestDisputed: extra?.disputed ?? false,
          displayStatus,
        });
      } catch (error) {
        items.push({
          index: itemRefs[i].index,
          itemID: itemRefs[i].itemID,
          descriptorError: error instanceof Error ? error.message : "Descriptor could not be decoded",
          rawData,
          status,
          numberOfRequests: Number(numRequests),
          requestDisputed: extra?.disputed ?? false,
          displayStatus,
        });
      }
    });
  }

  const isLoading = countLoading || idsLoading || infoLoading || extraLoading;
  let error = (countError || idsError || infoError || extraError) as Error | null;
  if (!error && !countIsSafe) {
    error = new Error("Registry itemCount exceeds the frontend's safe enumeration range");
  }
  if (!error && invalidStatus !== null) {
    error = new Error(`Registry returned an unknown item status: ${invalidStatus}`);
  }
  if (!error && !isLoading && idResults && (
    idResults.length !== count || idResults.some((result) => result.status !== "success")
  )) {
    error = new Error("Registry enumeration is incomplete because one or more itemList reads failed");
  }
  if (!error && !isLoading && infoResults && (
    infoResults.length !== itemRefs.length || infoResults.some((result) => result.status !== "success")
  )) {
    error = new Error("Registry view is incomplete because one or more getItemInfo reads failed");
  }
  if (!error && !isLoading && extraResults?.some((result) => result.status !== "success")) {
    error = new Error("Registry lifecycle details are incomplete because one or more request reads failed");
  }

  return { data: items, isLoading, error };
}
