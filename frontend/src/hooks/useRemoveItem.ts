import { useWriteContract, useWaitForTransactionReceipt } from "wagmi";
import { generalizedTcrAbi } from "../abi/GeneralizedTCR";
import { REGISTRY_ADDRESS } from "../config/registry";
import { removalDeposit } from "../lib/deposits";
import type { RegistryParams } from "../types";

export function useRemoveItem() {
  const { writeContract, data: hash, isPending, error, reset } = useWriteContract();
  const { isLoading: isConfirming, isSuccess } = useWaitForTransactionReceipt({ hash });

  function remove(itemID: `0x${string}`, evidence: string, params: RegistryParams) {
    const value = removalDeposit(params);
    writeContract({
      address: REGISTRY_ADDRESS,
      abi: generalizedTcrAbi,
      functionName: "removeItem",
      args: [itemID, evidence],
      value,
    });
  }

  return { remove, hash, isPending, isConfirming, isSuccess, error, reset };
}
