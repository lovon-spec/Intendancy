import { useState } from "react";
import { useAccount } from "wagmi";
import { useRegistryParams } from "../../hooks/useRegistryParams";
import { useRemoveItem } from "../../hooks/useRemoveItem";
import { DepositInfo } from "./DepositInfo";
import { TransactionStatus } from "../wallet/TransactionStatus";

interface RemoveFormProps {
  itemID: `0x${string}`;
}

export function RemoveForm({ itemID }: RemoveFormProps) {
  const { isConnected } = useAccount();
  const { data: params } = useRegistryParams();
  const { remove, hash, isPending, isConfirming, isSuccess, error, reset } = useRemoveItem();
  const [evidence, setEvidence] = useState("");

  function handleSubmit(e: React.FormEvent) {
    e.preventDefault();
    if (!params) return;
    reset();
    remove(itemID, evidence || "/ipfs/no-evidence", params);
  }

  return (
    <form onSubmit={handleSubmit} className="space-y-4">
      <div>
        <label className="block text-sm font-medium text-gray-700 mb-1">Reason for removal</label>
        <textarea
          value={evidence}
          onChange={(e) => setEvidence(e.target.value)}
          rows={3}
          placeholder="Explain why this entry should be removed..."
          className="w-full px-3 py-2 border border-gray-300 rounded-md text-sm focus:outline-none focus:ring-2 focus:ring-blue-500"
        />
      </div>

      {params && (
        <DepositInfo
          label="Removal deposit (returned if removal succeeds)"
          baseDeposit={params.removalBaseDeposit}
          arbitrationCost={params.arbitrationCost}
        />
      )}

      <TransactionStatus hash={hash} isPending={isPending} isConfirming={isConfirming} isSuccess={isSuccess} error={error} />

      <button
        type="submit"
        disabled={!isConnected || isPending || isConfirming}
        className="w-full py-2.5 px-4 text-sm font-medium text-white bg-orange-600 rounded-md hover:bg-orange-700 disabled:opacity-50 disabled:cursor-not-allowed"
      >
        {isPending ? "Confirm in wallet..." : "Request Removal"}
      </button>
    </form>
  );
}
