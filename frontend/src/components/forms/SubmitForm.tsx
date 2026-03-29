import { useState } from "react";
import { useAccount } from "wagmi";
import { CATEGORIES, RUNTIMES } from "../../config/registry";
import { useRegistryParams } from "../../hooks/useRegistryParams";
import { useSubmitItem } from "../../hooks/useSubmitItem";
import { DepositInfo } from "./DepositInfo";
import { TransactionStatus } from "../wallet/TransactionStatus";
import type { ItemFields } from "../../types";

const SOURCE_LOCATOR_REGEX = /^https?:\/\/.+@[0-9a-f]{40}$/i;

export function SubmitForm() {
  const { isConnected } = useAccount();
  const { data: params } = useRegistryParams();
  const { submit, hash, isPending, isConfirming, isSuccess, error, reset } = useSubmitItem();

  const [name, setName] = useState("");
  const [sourceLocator, setSourceLocator] = useState("");
  const [category, setCategory] = useState<string>("skill");
  const [selectedRuntimes, setSelectedRuntimes] = useState<string[]>(["generic"]);
  const [description, setDescription] = useState("");
  const [validationError, setValidationError] = useState("");

  function toggleRuntime(rt: string) {
    setSelectedRuntimes((prev) =>
      prev.includes(rt) ? prev.filter((r) => r !== rt) : [...prev, rt]
    );
  }

  function handleSubmit(e: React.FormEvent) {
    e.preventDefault();
    setValidationError("");
    reset();

    if (!name.trim()) { setValidationError("Name is required"); return; }
    if (!SOURCE_LOCATOR_REGEX.test(sourceLocator)) {
      setValidationError("Source Locator must be a URL followed by @ and a 40-char hex commit hash");
      return;
    }
    if (selectedRuntimes.length === 0) { setValidationError("Select at least one runtime"); return; }
    if (!description.trim()) { setValidationError("Description is required"); return; }
    if (!params) { setValidationError("Loading registry parameters..."); return; }

    const fields: ItemFields = {
      name: name.trim(),
      sourceType: "git",
      sourceLocator: sourceLocator.trim(),
      category: category as ItemFields["category"],
      runtimes: selectedRuntimes.join(","),
      description: description.trim(),
    };

    submit(fields, params);
  }

  return (
    <form onSubmit={handleSubmit} className="space-y-4">
      <div>
        <label className="block text-sm font-medium text-gray-700 mb-1">Name</label>
        <input
          type="text"
          value={name}
          onChange={(e) => setName(e.target.value)}
          placeholder="my-awesome-skill"
          className="w-full px-3 py-2 border border-gray-300 rounded-md text-sm focus:outline-none focus:ring-2 focus:ring-blue-500"
        />
      </div>

      <div>
        <label className="block text-sm font-medium text-gray-700 mb-1">Source Locator</label>
        <input
          type="text"
          value={sourceLocator}
          onChange={(e) => setSourceLocator(e.target.value)}
          placeholder="https://github.com/org/repo@abc123..."
          className="w-full px-3 py-2 border border-gray-300 rounded-md text-sm font-mono focus:outline-none focus:ring-2 focus:ring-blue-500"
        />
        <p className="text-xs text-gray-500 mt-1">Git repository URL followed by @ and the full 40-character commit hash</p>
      </div>

      <div>
        <label className="block text-sm font-medium text-gray-700 mb-1">Category</label>
        <select
          value={category}
          onChange={(e) => setCategory(e.target.value)}
          className="w-full px-3 py-2 border border-gray-300 rounded-md text-sm bg-white"
        >
          {CATEGORIES.map((c) => (
            <option key={c} value={c}>{c}</option>
          ))}
        </select>
      </div>

      <div>
        <label className="block text-sm font-medium text-gray-700 mb-1">Supported Runtimes</label>
        <div className="flex flex-wrap gap-2">
          {RUNTIMES.map((rt) => (
            <button
              key={rt}
              type="button"
              onClick={() => toggleRuntime(rt)}
              className={`px-3 py-1 rounded-full text-xs border transition-colors ${
                selectedRuntimes.includes(rt)
                  ? "bg-blue-100 border-blue-400 text-blue-800"
                  : "bg-gray-50 border-gray-300 text-gray-600 hover:bg-gray-100"
              }`}
            >
              {rt}
            </button>
          ))}
        </div>
      </div>

      <div>
        <label className="block text-sm font-medium text-gray-700 mb-1">Description</label>
        <textarea
          value={description}
          onChange={(e) => setDescription(e.target.value)}
          rows={3}
          placeholder="Brief description of what this entry does..."
          className="w-full px-3 py-2 border border-gray-300 rounded-md text-sm focus:outline-none focus:ring-2 focus:ring-blue-500"
        />
      </div>

      {params && (
        <DepositInfo
          label="Submission deposit (refunded if accepted)"
          baseDeposit={params.submissionBaseDeposit}
          arbitrationCost={params.arbitrationCost}
        />
      )}

      {validationError && (
        <div className="p-3 bg-red-50 border border-red-200 rounded-md text-sm text-red-700">
          {validationError}
        </div>
      )}

      <TransactionStatus hash={hash} isPending={isPending} isConfirming={isConfirming} isSuccess={isSuccess} error={error} />

      <button
        type="submit"
        disabled={!isConnected || isPending || isConfirming}
        className="w-full py-2.5 px-4 text-sm font-medium text-white bg-blue-600 rounded-md hover:bg-blue-700 disabled:opacity-50 disabled:cursor-not-allowed"
      >
        {!isConnected ? "Connect wallet first" : isPending ? "Confirm in wallet..." : isConfirming ? "Confirming..." : "Submit Entry"}
      </button>
    </form>
  );
}
