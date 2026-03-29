interface TransactionStatusProps {
  hash?: `0x${string}`;
  isPending: boolean;
  isConfirming: boolean;
  isSuccess: boolean;
  error: Error | null;
}

export function TransactionStatus({ hash, isPending, isConfirming, isSuccess, error }: TransactionStatusProps) {
  if (error) {
    return (
      <div className="p-3 bg-red-50 border border-red-200 rounded-md text-sm text-red-700">
        Transaction failed: {error.message.slice(0, 200)}
      </div>
    );
  }
  if (isPending) {
    return (
      <div className="p-3 bg-yellow-50 border border-yellow-200 rounded-md text-sm text-yellow-700">
        Waiting for wallet confirmation...
      </div>
    );
  }
  if (isConfirming) {
    return (
      <div className="p-3 bg-blue-50 border border-blue-200 rounded-md text-sm text-blue-700">
        Confirming transaction...
        {hash && <span className="block font-mono text-xs mt-1">{hash}</span>}
      </div>
    );
  }
  if (isSuccess) {
    return (
      <div className="p-3 bg-green-50 border border-green-200 rounded-md text-sm text-green-700">
        Transaction confirmed!
        {hash && <span className="block font-mono text-xs mt-1">{hash}</span>}
      </div>
    );
  }
  return null;
}
