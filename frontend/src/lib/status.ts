import { ItemStatus, type DisplayStatus } from "../types";

export function isItemStatus(status: number): status is ItemStatus {
  return (
    status === ItemStatus.Absent ||
    status === ItemStatus.Registered ||
    status === ItemStatus.RegistrationRequested ||
    status === ItemStatus.ClearingRequested
  );
}

/** Compute the requester-neutral display status for an item. */
export function getDisplayStatus(
  status: ItemStatus,
  latestRequestDisputed: boolean,
): DisplayStatus {
  switch (status) {
    case ItemStatus.Registered:
      return "registered";
    case ItemStatus.Absent:
      return "absent";
    case ItemStatus.RegistrationRequested:
      return latestRequestDisputed ? "disputed" : "pending-registration";
    // Every removal request is a uniform, non-installable safety flag. The
    // requester's identity does not grant special frontend or protocol status.
    case ItemStatus.ClearingRequested:
      return "flagged";
    default:
      return "absent";
  }
}

export const STATUS_LABELS: Record<DisplayStatus, string> = {
  registered: "Registered",
  "pending-registration": "Pending",
  disputed: "Disputed",
  flagged: "Removal Requested",
  absent: "Not Listed",
};

export const STATUS_COLORS: Record<DisplayStatus, string> = {
  registered: "bg-green-100 text-green-800",
  "pending-registration": "bg-yellow-100 text-yellow-800",
  disputed: "bg-red-100 text-red-800",
  flagged: "bg-red-200 text-red-900",
  absent: "bg-gray-100 text-gray-600",
};
