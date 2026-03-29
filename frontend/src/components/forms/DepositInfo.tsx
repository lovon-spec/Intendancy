import { formatXdai } from "../../lib/formatters";

interface DepositInfoProps {
  label: string;
  baseDeposit: bigint;
  arbitrationCost: bigint;
}

export function DepositInfo({ label, baseDeposit, arbitrationCost }: DepositInfoProps) {
  const total = baseDeposit + arbitrationCost;
  return (
    <div className="p-3 bg-gray-50 border border-gray-200 rounded-md text-sm">
      <div className="font-medium text-gray-700 mb-1">{label}</div>
      <div className="flex justify-between text-gray-500">
        <span>Base deposit</span>
        <span>{formatXdai(baseDeposit)}</span>
      </div>
      <div className="flex justify-between text-gray-500">
        <span>Arbitration cost</span>
        <span>{formatXdai(arbitrationCost)}</span>
      </div>
      <div className="flex justify-between font-medium text-gray-900 pt-1 border-t border-gray-200 mt-1">
        <span>Total required</span>
        <span>{formatXdai(total)}</span>
      </div>
    </div>
  );
}
