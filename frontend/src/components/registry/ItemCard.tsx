import { Link } from "react-router-dom";
import type { RegistryItem } from "../../types";
import { StatusBadge } from "./StatusBadge";
import { CategoryBadge } from "./CategoryBadge";
import { RuntimeTags } from "./RuntimeTags";

export function ItemCard({ item }: { item: RegistryItem }) {
  return (
    <Link
      to={`/item/${item.itemID}`}
      className="block border border-gray-200 rounded-lg p-4 hover:border-gray-400 hover:shadow-sm transition-all"
    >
      <div className="flex items-start justify-between gap-2 mb-2">
        <h3 className="font-semibold text-gray-900 text-sm truncate">{item.fields.name}</h3>
        <StatusBadge status={item.displayStatus} />
      </div>
      <div className="flex items-center gap-2 mb-2">
        <CategoryBadge category={item.fields.category} />
        <RuntimeTags runtimes={item.fields.runtimes} />
      </div>
      <p className="text-xs text-gray-500 line-clamp-2">{item.fields.description}</p>
    </Link>
  );
}
