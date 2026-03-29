import { useReadContracts, useReadContract } from "wagmi";
import { generalizedTcrAbi } from "../abi/GeneralizedTCR";
import { arbitratorAbi } from "../abi/IArbitrator";
import { REGISTRY_ADDRESS } from "../config/registry";
import type { RegistryParams } from "../types";

export function useRegistryParams(): { data: RegistryParams | undefined; isLoading: boolean } {
  const contract = { address: REGISTRY_ADDRESS, abi: generalizedTcrAbi } as const;

  const { data: params, isLoading: paramsLoading } = useReadContracts({
    contracts: [
      { ...contract, functionName: "submissionBaseDeposit" },
      { ...contract, functionName: "removalBaseDeposit" },
      { ...contract, functionName: "submissionChallengeBaseDeposit" },
      { ...contract, functionName: "removalChallengeBaseDeposit" },
      { ...contract, functionName: "challengePeriodDuration" },
      { ...contract, functionName: "arbitrator" },
      { ...contract, functionName: "arbitratorExtraData" },
    ],
  });

  const arbitratorAddr = params?.[5]?.result as `0x${string}` | undefined;
  const extraData = params?.[6]?.result as `0x${string}` | undefined;

  const { data: arbCost, isLoading: arbLoading } = useReadContract({
    address: arbitratorAddr,
    abi: arbitratorAbi,
    functionName: "arbitrationCost",
    args: [extraData ?? "0x"],
    query: { enabled: !!arbitratorAddr && !!extraData },
  });

  if (paramsLoading || arbLoading || !params || params.some((p) => p.status !== "success")) {
    return { data: undefined, isLoading: true };
  }

  return {
    data: {
      submissionBaseDeposit: params[0].result as bigint,
      removalBaseDeposit: params[1].result as bigint,
      submissionChallengeBaseDeposit: params[2].result as bigint,
      removalChallengeBaseDeposit: params[3].result as bigint,
      challengePeriodDuration: params[4].result as bigint,
      arbitrationCost: (arbCost as bigint) ?? 0n,
    },
    isLoading: false,
  };
}
