import { Link } from "react-router-dom";
import { SubmitForm } from "../components/forms/SubmitForm";

export function SubmitPage() {
  return (
    <div className="max-w-xl mx-auto space-y-6">
      <Link to="/" className="text-sm text-blue-600 hover:underline">&larr; Back to registry</Link>
      <div>
        <h1 className="text-2xl font-bold text-gray-900">Submit Entry</h1>
        <p className="text-sm text-gray-500 mt-1">
          Submit an AI agent skill, plugin, or convention to the registry.
          A deposit is required and will be refunded if the entry is accepted.
        </p>
      </div>
      <div className="border border-gray-200 rounded-lg p-5">
        <SubmitForm />
      </div>
    </div>
  );
}
