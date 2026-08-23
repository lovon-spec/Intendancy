import { useMemo, useState } from "react";
import { Link } from "react-router-dom";
import { useRegistryItems } from "../hooks/useRegistryItems";
import { ItemCard } from "../components/registry/ItemCard";
import { FilterBar } from "../components/registry/FilterBar";

export function RegistryPage() {
  const { data: items, isLoading, error } = useRegistryItems();
  const [search, setSearch] = useState("");
  const [status, setStatus] = useState("");

  const filtered = useMemo(() => {
    return items.filter((item) => {
      const searchableName = item.fields?.name || item.itemID;
      if (search && !searchableName.toLowerCase().includes(search.toLowerCase())) return false;
      if (status === "disputed" && !item.requestDisputed) return false;
      if (status && status !== "disputed" && item.displayStatus !== status) return false;
      // Hide absent items by default
      if (!status && item.displayStatus === "absent") return false;
      return true;
    });
  }, [items, search, status]);

  return (
    <div className="space-y-6">
      <div className="flex items-center justify-between">
        <div>
          <h1 className="text-2xl font-bold text-gray-900">Registry</h1>
          <p className="text-sm text-gray-500 mt-1">
            Community-curated AI agent skills
          </p>
        </div>
        <Link
          to="/submit"
          className="px-4 py-2 text-sm font-medium text-white bg-blue-600 rounded-md hover:bg-blue-700"
        >
          Submit Entry
        </Link>
      </div>

      <FilterBar
        search={search} onSearchChange={setSearch}
        status={status} onStatusChange={setStatus}
      />

      {isLoading && (
        <div className="text-center py-12 text-gray-500">Loading registry...</div>
      )}

      {error && (
        <div className="p-4 bg-red-50 border border-red-200 rounded-md text-sm text-red-700">
          Failed to load registry: {error.message}
        </div>
      )}

      {!isLoading && !error && filtered.length === 0 && (
        <div className="text-center py-12 text-gray-500">
          {items.length === 0 ? "No entries yet. Be the first to submit!" : "No entries match your filters."}
        </div>
      )}

      {!isLoading && !error && (
        <div className="grid gap-3 sm:grid-cols-2">
          {filtered.map((item) => (
            <ItemCard key={item.itemID} item={item} />
          ))}
        </div>
      )}
    </div>
  );
}
