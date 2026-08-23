import { useReadContract, useReadContracts } from "wagmi";
import { generalizedTcrAbi } from "../abi/GeneralizedTCR";
import { REGISTRY_ADDRESS } from "../config/registry";
import { decodeItem } from "../lib/encoder";
import { getItemValidationError } from "../lib/schema";
import { getDisplayStatus, isItemStatus } from "../lib/status";
import { ItemStatus, type ItemFields, type RequestInfo } from "../types";

const contract = { address: REGISTRY_ADDRESS, abi: generalizedTcrAbi } as const;

export function useItemDetail(itemID: `0x${string}`) {
  const {
    data: itemInfo,
    isLoading: itemLoading,
    error: itemError,
  } = useReadContract({
    ...contract,
    functionName: "getItemInfo",
    args: [itemID],
  });

  const rawData = itemInfo?.[0] as `0x${string}` | undefined;
  const rawStatus = itemInfo?.[1] as number | undefined;
  const statusIsValid = rawStatus !== undefined && isItemStatus(rawStatus);
  // The fallback is never rendered: an absent/invalid result becomes an error
  // below, and the page suppresses status and actions on any incomplete read.
  const status = statusIsValid ? rawStatus : ItemStatus.Absent;
  const rawNumRequests = itemInfo?.[2] as bigint | undefined;
  const requestCountIsSafe =
    rawNumRequests === undefined || rawNumRequests <= BigInt(Number.MAX_SAFE_INTEGER);
  const numRequests = requestCountIsSafe ? Number(rawNumRequests ?? 0n) : 0;

  let fields: ItemFields | undefined;
  let descriptorError: string | undefined;
  try {
    if (rawData && rawData !== "0x") {
      fields = decodeItem(rawData);
      descriptorError = getItemValidationError(fields) ?? undefined;
    }
  } catch (caught) {
    descriptorError = caught instanceof Error ? caught.message : "Descriptor could not be decoded";
  }

  const requestContracts = Array.from({ length: numRequests }, (_, index) => ({
    ...contract,
    functionName: "getRequestInfo" as const,
    args: [itemID, BigInt(index)] as const,
  }));

  const {
    data: requestResults,
    isLoading: requestsLoading,
    error: requestsError,
  } = useReadContracts({
    contracts: requestContracts,
    query: { enabled: numRequests > 0 },
  });

  const requestsComplete =
    numRequests === 0 ||
    (requestResults?.length === numRequests &&
      requestResults.every((result) => result.status === "success"));

  const requests: RequestInfo[] = requestsComplete
    ? (requestResults ?? []).map((result) => {
        if (result.status !== "success") {
          throw new Error("Unreachable incomplete request result");
        }
        const value = result.result as readonly [
          boolean,
          bigint,
          bigint,
          boolean,
          readonly [`0x${string}`, `0x${string}`, `0x${string}`],
          bigint,
          number,
          string,
          string,
          bigint,
        ];
        return {
          disputed: value[0],
          disputeID: value[1],
          submissionTime: value[2],
          resolved: value[3],
          parties: value[4],
          numberOfRounds: value[5],
          ruling: value[6],
        };
      })
    : [];

  const latestRequest = requests.at(-1);
  const displayStatus = getDisplayStatus(status, latestRequest?.disputed ?? false);
  const isLoading = itemLoading || requestsLoading;

  let error = (itemError || requestsError) as Error | null;
  if (!error && !isLoading && !itemInfo) {
    error = new Error("Registry item information was not returned");
  }
  if (!error && itemInfo && !statusIsValid) {
    error = new Error(`Registry returned an unknown item status: ${String(rawStatus)}`);
  }
  if (!error && !requestCountIsSafe) {
    error = new Error("Request count exceeds the frontend's safe range");
  }
  if (!error && !isLoading && !requestsComplete) {
    error = new Error("Item lifecycle is incomplete because one or more request reads failed");
  }

  return {
    fields,
    descriptorError,
    rawData,
    status,
    displayStatus,
    requests,
    numRequests,
    isLoading,
    error,
  };
}
