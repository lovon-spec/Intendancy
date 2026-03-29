const CATEGORY_COLORS: Record<string, string> = {
  skill: "bg-blue-100 text-blue-800",
  plugin: "bg-purple-100 text-purple-800",
  convention: "bg-teal-100 text-teal-800",
};

export function CategoryBadge({ category }: { category: string }) {
  const color = CATEGORY_COLORS[category] ?? "bg-gray-100 text-gray-800";
  return (
    <span className={`inline-flex items-center px-2.5 py-0.5 rounded-full text-xs font-medium ${color}`}>
      {category}
    </span>
  );
}
