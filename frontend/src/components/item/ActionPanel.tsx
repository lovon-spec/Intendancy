import { useState } from "react";
import { useAccount } from "wagmi";
import { ItemStatus } from "../../types";
import { useExecuteRequest } from "../../hooks/useExecuteRequest";
import { ChallengeForm } from "../forms/ChallengeForm";
import { RemoveForm } from "../forms/RemoveForm";
import { TransactionStatus } from "../wallet/TransactionStatus";
import type { RequestInfo } from "../../types";

interface ActionPanelProps {
  itemID: `0x${string}`;
  status: ItemStatus;
  latestRequest?: RequestInfo;
  challengePeriodDuration: bigint;
}

export function ActionPanel({ itemID, status, latestRequest, challengePeriodDuration }: ActionPanelProps) {
  const { isConnected } = useAccount();
  const [showChallenge, setShowChallenge] = useState(false);
  const [showRemove, setShowRemove] = useState(false);
  const { execute, hash, isPending, isConfirming, isSuccess, error } = useExecuteRequest();

  if (!isConnected) {
    return <p className="text-sm text-gray-500">Connect your wallet to interact with this entry.</p>;
  }

  const canChallenge =
    (status === ItemStatus.RegistrationRequested || status === ItemStatus.ClearingRequested) &&
    latestRequest && !latestRequest.disputed && !latestRequest.resolved;

  const canExecute =
    (status === ItemStatus.RegistrationRequested || status === ItemStatus.ClearingRequested) &&
    latestRequest && !latestRequest.disputed && !latestRequest.resolved &&
    BigInt(Math.floor(Date.now() / 1000)) > latestRequest.submissionTime + challengePeriodDuration;

  const canRemove = status === ItemStatus.Registered;

  return (
    <div className="space-y-3">
      <h3 className="text-sm font-medium text-gray-700">Actions</h3>

      <div className="flex flex-wrap gap-2">
        {canExecute && (
          <button
            onClick={() => execute(itemID)}
            disabled={isPending || isConfirming}
            className="px-4 py-2 text-sm font-medium text-white bg-green-600 rounded-md hover:bg-green-700 disabled:opacity-50"
          >
            Execute Request
          </button>
        )}
        {canChallenge && !canExecute && (
          <button
            onClick={() => setShowChallenge(!showChallenge)}
            className="px-4 py-2 text-sm font-medium text-white bg-red-600 rounded-md hover:bg-red-700"
          >
            {showChallenge ? "Cancel" : "Challenge"}
          </button>
        )}
        {canRemove && (
          <button
            onClick={() => setShowRemove(!showRemove)}
            className="px-4 py-2 text-sm font-medium text-white bg-orange-600 rounded-md hover:bg-orange-700"
          >
            {showRemove ? "Cancel" : "Request Removal"}
          </button>
        )}
      </div>

      <TransactionStatus hash={hash} isPending={isPending} isConfirming={isConfirming} isSuccess={isSuccess} error={error} />

      {showChallenge && canChallenge && (
        <ChallengeForm itemID={itemID} itemStatus={status} />
      )}
      {showRemove && canRemove && (
        <RemoveForm itemID={itemID} />
      )}
    </div>
  );
}
