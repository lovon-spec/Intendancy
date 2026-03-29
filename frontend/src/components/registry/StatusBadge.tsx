import { STATUS_LABELS, STATUS_COLORS } from "../../lib/status";
import type { DisplayStatus } from "../../types";

export function StatusBadge({ status }: { status: DisplayStatus }) {
  return (
    <span className={`inline-flex items-center px-2.5 py-0.5 rounded-full text-xs font-medium ${STATUS_COLORS[status]}`}>
      {STATUS_LABELS[status]}
    </span>
  );
}
