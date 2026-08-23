import type { ItemFields as Fields } from "../../types";
import { originHref } from "../../lib/schema";
import { RuntimeTags } from "../registry/RuntimeTags";

export function ItemFieldsDisplay({ fields }: { fields: Fields }) {
  const originUrl = originHref(fields.origin);

  return (
    <div className="space-y-3">
      <div>
        <dt className="text-xs font-medium text-gray-500 uppercase">Description</dt>
        <dd className="mt-0.5 text-sm text-gray-700">{fields.description}</dd>
      </div>
      <div>
        <dt className="text-xs font-medium text-gray-500 uppercase">Tree CID</dt>
        <dd className="mt-0.5">
          <a
            href={`https://dweb.link/ipfs/${encodeURIComponent(fields.treeCid)}`}
            target="_blank"
            rel="noopener noreferrer"
            className="text-sm font-mono text-blue-600 hover:underline break-all"
          >
            {fields.treeCid}
          </a>
        </dd>
      </div>
      <div>
        <dt className="text-xs font-medium text-gray-500 uppercase">Runtimes</dt>
        <dd className="mt-1"><RuntimeTags runtimes={fields.runtimes} /></dd>
      </div>
      {fields.origin && (
        <div>
          <dt className="text-xs font-medium text-gray-500 uppercase">Origin</dt>
          <dd className="mt-0.5">
            {originUrl ? (
              <a href={originUrl} target="_blank" rel="noopener noreferrer" className="text-sm font-mono text-blue-600 hover:underline break-all">
                {fields.origin}
              </a>
            ) : (
              <span className="text-sm font-mono break-all">{fields.origin}</span>
            )}
          </dd>
        </div>
      )}
    </div>
  );
}
