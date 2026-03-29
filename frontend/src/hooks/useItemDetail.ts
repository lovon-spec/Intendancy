import { useReadContract, useReadContracts } from "wagmi";
import { generalizedTcrAbi } from "../abi/GeneralizedTCR";
import { REGISTRY_ADDRESS } from "../config/registry";
import { decodeItem } from "../lib/encoder";
import { getDisplayStatus } from "../lib/status";
import { ItemStatus, type DisplayStatus, type ItemFields, type RequestInfo } from "../types";

const contract = { address: REGISTRY_ADDRESS, abi: generalizedTcrAbi } as const;

export function useItemDetail(itemID: `0x${string}`) {
  const { data: itemInfo, isLoading } = useReadContract({
    ...contract,
    functionName: "getItemInfo",
    args: [itemID],
  });

  const rawData = itemInfo?.[0] as `0x${string}` | undefined;
  const status = (itemInfo?.[1] as number | undefined) ?? 0;
  const numRequests = Number(itemInfo?.[2] ?? 0);

  let fields: ItemFields | undefined;
  try {
    if (rawData && rawData !== "0x") fields = decodeItem(rawData);
  } catch { /* not our schema */ }

  // Fetch all requests
  const requestContracts = Array.from({ length: numRequests }, (_, i) => ({
    ...contract,
    functionName: "getRequestInfo" as const,
    args: [itemID, BigInt(i)] as const,
  }));

  const { data: requestResults } = useReadContracts({
    contracts: requestContracts,
    query: { enabled: numRequests > 0 },
  });

  const requests: RequestInfo[] = (requestResults ?? [])
    .filter((r) => r.status === "success")
    .map((r) => {
      const res = r.result as readonly [boolean, bigint, bigint, boolean, readonly [`0x${string}`, `0x${string}`, `0x${string}`], bigint, number, string, string, bigint];
      return {
        disputed: res[0],
        disputeID: res[1],
        submissionTime: res[2],
        resolved: res[3],
        parties: res[4],
        numberOfRounds: res[5],
        ruling: res[6],
      };
    });

  const latestRequest = requests.length > 0 ? requests[requests.length - 1] : undefined;
  const displayStatus: DisplayStatus = getDisplayStatus(
    status as ItemStatus,
    latestRequest?.disputed ?? false,
    latestRequest?.parties[1],
  );

  return {
    fields,
    rawData,
    status: status as ItemStatus,
    displayStatus,
    requests,
    numRequests,
    isLoading,
  };
}
