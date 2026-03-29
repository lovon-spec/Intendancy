import { ItemStatus, type DisplayStatus } from "../types";
import { GUARDIAN_ADDRESS } from "../config/registry";

const SIX_MONTHS_SECONDS = 180 * 24 * 60 * 60; // ~6 months

/**
 * Compute the display status for an item.
 * @param registeredSince - timestamp (seconds) when the item was first registered.
 *   Pass undefined if not known or not applicable.
 */
export function getDisplayStatus(
  status: ItemStatus,
  latestRequestDisputed: boolean,
  latestRequester?: `0x${string}`,
  registeredSince?: bigint,
): DisplayStatus {
  switch (status) {
    case ItemStatus.Registered: {
      if (registeredSince) {
        const now = BigInt(Math.floor(Date.now() / 1000));
        if (now - registeredSince >= SIX_MONTHS_SECONDS) {
          return "long-standing";
        }
      }
      return "registered";
    }
    case ItemStatus.Absent:
      return "absent";
    case ItemStatus.RegistrationRequested:
      return latestRequestDisputed ? "disputed" : "pending-registration";
    case ItemStatus.ClearingRequested: {
      if (latestRequestDisputed) return "disputed";
      if (
        latestRequester &&
        latestRequester.toLowerCase() === GUARDIAN_ADDRESS.toLowerCase() &&
        GUARDIAN_ADDRESS !== "0x0000000000000000000000000000000000000000"
      ) {
        return "flagged";
      }
      return "pending-removal";
    }
    default:
      return "absent";
  }
}

export const STATUS_LABELS: Record<DisplayStatus, string> = {
  registered: "Registered",
  "long-standing": "Long-standing",
  "pending-registration": "Pending",
  "pending-removal": "Removal Requested",
  disputed: "Disputed",
  flagged: "Flagged",
  absent: "Not Listed",
};

export const STATUS_COLORS: Record<DisplayStatus, string> = {
  registered: "bg-green-100 text-green-800",
  "long-standing": "bg-emerald-100 text-emerald-800",
  "pending-registration": "bg-yellow-100 text-yellow-800",
  "pending-removal": "bg-orange-100 text-orange-800",
  disputed: "bg-red-100 text-red-800",
  flagged: "bg-red-200 text-red-900",
  absent: "bg-gray-100 text-gray-600",
};
