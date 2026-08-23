import { useParams, Link } from "react-router-dom";
import { useItemDetail } from "../hooks/useItemDetail";
import { useRegistryParams } from "../hooks/useRegistryParams";
import { StatusBadge } from "../components/registry/StatusBadge";
import { ItemFieldsDisplay } from "../components/item/ItemFields";
import { RequestTimeline } from "../components/item/RequestTimeline";
import { ActionPanel } from "../components/item/ActionPanel";
import { ItemStatus } from "../types";

export function ItemPage() {
  const { itemId } = useParams<{ itemId: string }>();
  const detail = useItemDetail(itemId as `0x${string}`);
  const { data: params } = useRegistryParams();

  if (detail.isLoading) {
    return <div className="text-center py-12 text-gray-500">Loading...</div>;
  }

  if (detail.error) {
    return (
      <div className="space-y-6">
        <Link to="/" className="text-sm text-blue-600 hover:underline">&larr; Back to registry</Link>
        <div className="p-4 bg-red-50 border border-red-200 rounded-md text-sm text-red-700">
          Failed to load a complete item history: {detail.error.message}
        </div>
      </div>
    );
  }

  const latestRequest = detail.requests.length > 0 ? detail.requests[detail.requests.length - 1] : undefined;

  return (
    <div className="space-y-6">
      <Link to="/" className="text-sm text-blue-600 hover:underline">&larr; Back to registry</Link>

      <div className="flex items-start justify-between gap-4">
        <h1 className="text-2xl font-bold text-gray-900">
          {detail.fields?.name ||
            (detail.status === ItemStatus.Absent && detail.rawData === "0x"
              ? "Unknown item"
              : "Malformed descriptor")}
        </h1>
        <StatusBadge status={detail.displayStatus} />
      </div>

      {detail.descriptorError && (
        <div className="p-4 bg-red-50 border border-red-200 rounded-md text-sm text-red-800">
          <p className="font-medium">This descriptor violates the V1 schema.</p>
          <p className="mt-1">{detail.descriptorError}</p>
          {detail.rawData && (
            <p className="mt-2 font-mono text-xs break-all">Raw descriptor: {detail.rawData}</p>
          )}
        </div>
      )}

      {detail.fields && (
        <div className="border border-gray-200 rounded-lg p-5">
          <ItemFieldsDisplay fields={detail.fields} />
        </div>
      )}

      {params && (
        <div className="border border-gray-200 rounded-lg p-5">
          <ActionPanel
            itemID={itemId as `0x${string}`}
            status={detail.status}
            latestRequest={latestRequest}
            challengePeriodDuration={params.challengePeriodDuration}
          />
        </div>
      )}

      <div className="border border-gray-200 rounded-lg p-5">
        <RequestTimeline requests={detail.requests} />
      </div>

      <div className="text-xs text-gray-400 font-mono break-all">
        Item ID: {itemId}
      </div>
    </div>
  );
}
