import type { ItemFields as Fields } from "../../types";
import { parseSourceLocator } from "../../lib/formatters";
import { CategoryBadge } from "../registry/CategoryBadge";
import { RuntimeTags } from "../registry/RuntimeTags";

export function ItemFieldsDisplay({ fields }: { fields: Fields }) {
  const source = parseSourceLocator(fields.sourceLocator);

  return (
    <div className="space-y-3">
      <div>
        <dt className="text-xs font-medium text-gray-500 uppercase">Source</dt>
        <dd className="mt-0.5">
          {source ? (
            <div>
              <a href={source.repoUrl} target="_blank" rel="noopener noreferrer" className="text-sm text-blue-600 hover:underline">
                {source.repoUrl}
              </a>
              <div className="font-mono text-xs text-gray-500 mt-0.5">
                @ {source.commitHash}
              </div>
            </div>
          ) : (
            <span className="text-sm font-mono">{fields.sourceLocator}</span>
          )}
        </dd>
      </div>
      <div className="flex gap-6">
        <div>
          <dt className="text-xs font-medium text-gray-500 uppercase">Category</dt>
          <dd className="mt-1"><CategoryBadge category={fields.category} /></dd>
        </div>
        <div>
          <dt className="text-xs font-medium text-gray-500 uppercase">Runtimes</dt>
          <dd className="mt-1"><RuntimeTags runtimes={fields.runtimes} /></dd>
        </div>
      </div>
      <div>
        <dt className="text-xs font-medium text-gray-500 uppercase">Description</dt>
        <dd className="mt-0.5 text-sm text-gray-700">{fields.description}</dd>
      </div>
    </div>
  );
}
