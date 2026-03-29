import { useEffect, useState } from "react";
import { Link } from "react-router-dom";
import Markdown from "react-markdown";

const POLICY_URL = import.meta.env.VITE_POLICY_URL || "/listing-policy.md";

export function PolicyPage() {
  const [content, setContent] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    fetch(POLICY_URL)
      .then((res) => {
        if (!res.ok) throw new Error(`Failed to fetch policy: ${res.status}`);
        return res.text();
      })
      .then(setContent)
      .catch((e) => setError(e.message));
  }, []);

  return (
    <div className="max-w-3xl mx-auto">
      <Link to="/" className="text-sm text-blue-600 hover:underline">&larr; Back to registry</Link>

      {error && (
        <div className="mt-4 p-4 bg-red-50 border border-red-200 rounded-md text-sm text-red-700">
          {error}
        </div>
      )}

      {!content && !error && (
        <div className="mt-4 text-center text-gray-500">Loading policy...</div>
      )}

      {content && (
        <div className="mt-4 prose prose-sm prose-gray max-w-none [&_h1]:text-2xl [&_h1]:font-bold [&_h1]:mb-4 [&_h2]:text-lg [&_h2]:font-semibold [&_h2]:mt-6 [&_h2]:mb-2 [&_h3]:text-base [&_h3]:font-semibold [&_h3]:mt-4 [&_h3]:mb-1 [&_p]:text-sm [&_p]:text-gray-700 [&_p]:mb-3 [&_li]:text-sm [&_li]:text-gray-700 [&_ul]:mb-3 [&_ol]:mb-3 [&_table]:text-xs [&_table]:w-full [&_th]:text-left [&_th]:p-2 [&_th]:bg-gray-50 [&_td]:p-2 [&_td]:border-t [&_code]:text-xs [&_code]:bg-gray-100 [&_code]:px-1 [&_code]:py-0.5 [&_code]:rounded [&_strong]:text-gray-900">
          <Markdown>{content}</Markdown>
        </div>
      )}

      <div className="mt-6 text-xs text-gray-400">
        Source: <a href={POLICY_URL} target="_blank" rel="noopener noreferrer" className="hover:underline">{POLICY_URL}</a>
      </div>
    </div>
  );
}
