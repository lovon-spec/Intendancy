import type { ItemFields as Fields } from "../../types";
import { SOURCE_TYPE_CONFIG, type SourceType } from "../../config/registry";
import { CategoryBadge } from "../registry/CategoryBadge";
import { RuntimeTags } from "../registry/RuntimeTags";

const SOURCE_TYPE_LABELS: Record<string, string> = {
  git: "Git Repository",
  npm: "npm Package",
  pypi: "PyPI Package",
  docker: "Docker Image",
  ipfs: "IPFS",
};

export function ItemFieldsDisplay({ fields }: { fields: Fields }) {
  const stConfig = SOURCE_TYPE_CONFIG[fields.sourceType as SourceType];
  const url = stConfig?.toUrl(fields.sourceLocator);

  return (
    <div className="space-y-3">
      <div>
        <dt className="text-xs font-medium text-gray-500 uppercase">
          Source ({SOURCE_TYPE_LABELS[fields.sourceType] ?? fields.sourceType})
        </dt>
        <dd className="mt-0.5">
          {url ? (
            <div>
              <a href={url} target="_blank" rel="noopener noreferrer" className="text-sm text-blue-600 hover:underline break-all">
                {fields.sourceLocator}
              </a>
            </div>
          ) : (
            <span className="text-sm font-mono break-all">{fields.sourceLocator}</span>
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
