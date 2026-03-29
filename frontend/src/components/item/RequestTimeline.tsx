import type { RequestInfo } from "../../types";
import { truncateAddress } from "../../lib/formatters";

export function RequestTimeline({ requests }: { requests: RequestInfo[] }) {
  if (requests.length === 0) return null;

  return (
    <div className="space-y-3">
      <h3 className="text-sm font-medium text-gray-700">Request History</h3>
      <div className="space-y-2">
        {requests.map((req, i) => (
          <div key={i} className="flex items-start gap-3 p-3 bg-gray-50 rounded-md text-sm">
            <div className="flex-shrink-0 w-6 h-6 rounded-full bg-gray-200 flex items-center justify-center text-xs font-medium">
              {i + 1}
            </div>
            <div className="flex-1">
              <div className="flex items-center gap-2 flex-wrap">
                <span className="font-medium">
                  {req.resolved ? "Resolved" : req.disputed ? "Disputed" : "Pending"}
                </span>
                {req.disputed && (
                  <span className="text-xs text-gray-500">
                    Dispute #{req.disputeID.toString()}
                  </span>
                )}
                {req.resolved && req.ruling !== 0 && (
                  <span className={`text-xs px-1.5 py-0.5 rounded ${req.ruling === 1 ? "bg-green-100 text-green-700" : "bg-red-100 text-red-700"}`}>
                    {req.ruling === 1 ? "Requester wins" : "Challenger wins"}
                  </span>
                )}
              </div>
              <div className="text-xs text-gray-500 mt-1">
                Requester: {truncateAddress(req.parties[1])}
                {req.parties[2] !== "0x0000000000000000000000000000000000000000" && (
                  <> | Challenger: {truncateAddress(req.parties[2])}</>
                )}
              </div>
              <div className="text-xs text-gray-400">
                Submitted: {new Date(Number(req.submissionTime) * 1000).toLocaleString()}
              </div>
            </div>
          </div>
        ))}
      </div>
    </div>
  );
}
