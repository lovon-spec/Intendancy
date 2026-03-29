import { useWriteContract, useWaitForTransactionReceipt } from "wagmi";
import { generalizedTcrAbi } from "../abi/GeneralizedTCR";
import { REGISTRY_ADDRESS } from "../config/registry";
import { encodeItem } from "../lib/encoder";
import { submissionDeposit } from "../lib/deposits";
import type { ItemFields, RegistryParams } from "../types";

export function useSubmitItem() {
  const { writeContract, data: hash, isPending, error, reset } = useWriteContract();
  const { isLoading: isConfirming, isSuccess } = useWaitForTransactionReceipt({ hash });

  function submit(fields: ItemFields, params: RegistryParams) {
    const encoded = encodeItem(fields);
    const value = submissionDeposit(params);
    writeContract({
      address: REGISTRY_ADDRESS,
      abi: generalizedTcrAbi,
      functionName: "addItem",
      args: [encoded],
      value,
    });
  }

  return { submit, hash, isPending, isConfirming, isSuccess, error, reset };
}
