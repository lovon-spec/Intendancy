import { useState } from "react";
import { useAccount } from "wagmi";
import { useRegistryParams } from "../../hooks/useRegistryParams";
import { useChallengeRequest } from "../../hooks/useChallengeRequest";
import { DepositInfo } from "./DepositInfo";
import { TransactionStatus } from "../wallet/TransactionStatus";
import { ItemStatus } from "../../types";

interface ChallengeFormProps {
  itemID: `0x${string}`;
  itemStatus: ItemStatus;
}

export function ChallengeForm({ itemID, itemStatus }: ChallengeFormProps) {
  const { isConnected } = useAccount();
  const { data: params } = useRegistryParams();
  const { challenge, hash, isPending, isConfirming, isSuccess, error, reset } = useChallengeRequest();
  const [evidence, setEvidence] = useState("");

  function handleSubmit(e: React.FormEvent) {
    e.preventDefault();
    if (!params) return;
    reset();
    challenge(itemID, evidence || "/ipfs/no-evidence", itemStatus, params);
  }

  const baseDeposit = itemStatus === ItemStatus.RegistrationRequested
    ? params?.submissionChallengeBaseDeposit ?? 0n
    : params?.removalChallengeBaseDeposit ?? 0n;

  return (
    <form onSubmit={handleSubmit} className="space-y-4">
      <div>
        <label className="block text-sm font-medium text-gray-700 mb-1">Evidence (IPFS URI or description)</label>
        <textarea
          value={evidence}
          onChange={(e) => setEvidence(e.target.value)}
          rows={3}
          placeholder="Explain why this entry should be rejected..."
          className="w-full px-3 py-2 border border-gray-300 rounded-md text-sm focus:outline-none focus:ring-2 focus:ring-blue-500"
        />
      </div>

      {params && (
        <DepositInfo
          label="Challenge deposit (returned if you win)"
          baseDeposit={baseDeposit}
          arbitrationCost={params.arbitrationCost}
        />
      )}

      <TransactionStatus hash={hash} isPending={isPending} isConfirming={isConfirming} isSuccess={isSuccess} error={error} />

      <button
        type="submit"
        disabled={!isConnected || isPending || isConfirming}
        className="w-full py-2.5 px-4 text-sm font-medium text-white bg-red-600 rounded-md hover:bg-red-700 disabled:opacity-50 disabled:cursor-not-allowed"
      >
        {isPending ? "Confirm in wallet..." : "Challenge"}
      </button>
    </form>
  );
}
