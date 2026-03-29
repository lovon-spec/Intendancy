export function RuntimeTags({ runtimes }: { runtimes: string }) {
  const tags = runtimes.split(",").map((r) => r.trim()).filter(Boolean);
  return (
    <div className="flex flex-wrap gap-1">
      {tags.map((tag) => (
        <span key={tag} className="inline-flex items-center px-2 py-0.5 rounded text-xs bg-gray-100 text-gray-700">
          {tag}
        </span>
      ))}
    </div>
  );
}
