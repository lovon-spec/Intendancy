import { useWriteContract, useWaitForTransactionReceipt } from "wagmi";
import { generalizedTcrAbi } from "../abi/GeneralizedTCR";
import { REGISTRY_ADDRESS } from "../config/registry";
import { challengeDeposit } from "../lib/deposits";
import type { RegistryParams } from "../types";
import { ItemStatus } from "../types";

export function useChallengeRequest() {
  const { writeContract, data: hash, isPending, error, reset } = useWriteContract();
  const { isLoading: isConfirming, isSuccess } = useWaitForTransactionReceipt({ hash });

  function challenge(
    itemID: `0x${string}`,
    evidence: string,
    itemStatus: ItemStatus,
    params: RegistryParams,
  ) {
    const value = challengeDeposit(params, itemStatus);
    writeContract({
      address: REGISTRY_ADDRESS,
      abi: generalizedTcrAbi,
      functionName: "challengeRequest",
      args: [itemID, evidence],
      value,
    });
  }

  return { challenge, hash, isPending, isConfirming, isSuccess, error, reset };
}
