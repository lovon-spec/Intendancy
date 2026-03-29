import { useWriteContract, useWaitForTransactionReceipt } from "wagmi";
import { generalizedTcrAbi } from "../abi/GeneralizedTCR";
import { REGISTRY_ADDRESS } from "../config/registry";

export function useExecuteRequest() {
  const { writeContract, data: hash, isPending, error, reset } = useWriteContract();
  const { isLoading: isConfirming, isSuccess } = useWaitForTransactionReceipt({ hash });

  function execute(itemID: `0x${string}`) {
    writeContract({
      address: REGISTRY_ADDRESS,
      abi: generalizedTcrAbi,
      functionName: "executeRequest",
      args: [itemID],
    });
  }

  return { execute, hash, isPending, isConfirming, isSuccess, error, reset };
}
