import { useState } from "react";
import { useAccount } from "wagmi";
import { SUGGESTED_RUNTIMES as RUNTIMES } from "../../config/registry";
import { useRegistryParams } from "../../hooks/useRegistryParams";
import { useSubmitItem } from "../../hooks/useSubmitItem";
import { getItemValidationError, canonicalRuntimes } from "../../lib/schema";
import { DepositInfo } from "./DepositInfo";
import { TransactionStatus } from "../wallet/TransactionStatus";
import type { ItemFields } from "../../types";

export function SubmitForm() {
  const { isConnected } = useAccount();
  const { data: params } = useRegistryParams();
  const { submit, hash, isPending, isConfirming, isSuccess, error, reset } = useSubmitItem();

  const [name, setName] = useState("");
  const [description, setDescription] = useState("");
  const [treeCid, setTreeCid] = useState("");
  const [selectedRuntimes, setSelectedRuntimes] = useState<string[]>(["generic"]);
  const [customRuntime, setCustomRuntime] = useState("");
  const [origin, setOrigin] = useState("");
  const [validationError, setValidationError] = useState("");

  // The policy reserves `generic` for skills with no runtime-specific features, alone.
  function toggleRuntime(rt: string) {
    setSelectedRuntimes((prev) => {
      if (prev.includes(rt)) return prev.filter((r) => r !== rt);
      if (rt === "generic") return ["generic"];
      return [...prev.filter((r) => r !== "generic"), rt];
    });
  }

  function addCustomRuntime() {
    const rt = customRuntime.trim().toLowerCase().replace(/[\s-]+/g, "_").replace(/[^a-z0-9_]/g, "");
    if (rt && rt !== "generic" && !selectedRuntimes.includes(rt)) {
      setSelectedRuntimes((prev) => [...prev.filter((r) => r !== "generic"), rt]);
    }
    setCustomRuntime("");
  }

  function handleSubmit(e: React.FormEvent) {
    e.preventDefault();
    setValidationError("");
    reset();

    const fields: ItemFields = {
      name,
      description,
      treeCid,
      runtimes: canonicalRuntimes(selectedRuntimes),
      origin,
      reserved: "",
    };

    const descriptorError = getItemValidationError(fields);
    if (descriptorError) { setValidationError(descriptorError); return; }
    if (!params) { setValidationError("Loading registry parameters..."); return; }

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
        <p className="text-xs text-gray-500 mt-1">Must be byte-identical to the SKILL.md frontmatter name</p>
      </div>

      <div>
        <label className="block text-sm font-medium text-gray-700 mb-1">Description</label>
        <textarea
          value={description}
          onChange={(e) => setDescription(e.target.value)}
          rows={3}
          placeholder="Exact description from SKILL.md frontmatter"
          className="w-full px-3 py-2 border border-gray-300 rounded-md text-sm focus:outline-none focus:ring-2 focus:ring-blue-500"
        />
        <p className="text-xs text-gray-500 mt-1">Must be byte-identical to the SKILL.md frontmatter description</p>
      </div>

      <div>
        <label className="block text-sm font-medium text-gray-700 mb-1">Tree CID</label>
        <input
          type="text"
          value={treeCid}
          onChange={(e) => setTreeCid(e.target.value)}
          placeholder="bafybeigdyrzt5sfp7udm7hu76uh7y26nf3efuylqabf3oclgtqy55fbzdi"
          className="w-full px-3 py-2 border border-gray-300 rounded-md text-sm font-mono focus:outline-none focus:ring-2 focus:ring-blue-500"
        />
        <p className="text-xs text-gray-500 mt-1">This form checks only canonical CIDv1 syntax (DAG-PB with SHA-256). Policy still requires you to upload and pin the complete UnixFS tree, verify its contents, size, and SKILL.md bindings, and keep it available throughout the submission period.</p>
      </div>

      <div>
        <label className="block text-sm font-medium text-gray-700 mb-1">Supported Runtimes</label>
        <div className="flex flex-wrap gap-2 mb-2">
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
        {selectedRuntimes.filter((rt) => !RUNTIMES.includes(rt as typeof RUNTIMES[number])).length > 0 && (
          <div className="flex flex-wrap gap-2 mb-2">
            {selectedRuntimes.filter((rt) => !RUNTIMES.includes(rt as typeof RUNTIMES[number])).map((rt) => (
              <button
                key={rt}
                type="button"
                onClick={() => toggleRuntime(rt)}
                className="px-3 py-1 rounded-full text-xs border bg-blue-100 border-blue-400 text-blue-800 transition-colors"
              >
                {rt} &times;
              </button>
            ))}
          </div>
        )}
        <div className="flex gap-2">
          <input
            type="text"
            value={customRuntime}
            onChange={(e) => setCustomRuntime(e.target.value)}
            onKeyDown={(e) => { if (e.key === "Enter") { e.preventDefault(); addCustomRuntime(); } }}
            placeholder="Add custom runtime..."
            className="flex-1 px-3 py-1.5 border border-gray-300 rounded-md text-xs focus:outline-none focus:ring-2 focus:ring-blue-500"
          />
          <button
            type="button"
            onClick={addCustomRuntime}
            disabled={!customRuntime.trim()}
            className="px-3 py-1.5 text-xs font-medium text-gray-700 border border-gray-300 rounded-md hover:bg-gray-50 disabled:opacity-50"
          >
            Add
          </button>
        </div>
        <p className="text-xs text-gray-500 mt-1">Click suggestions above or type a custom runtime name</p>
      </div>

      <div>
        <label className="block text-sm font-medium text-gray-700 mb-1">Origin <span className="font-normal text-gray-400">(optional)</span></label>
        <input
          type="text"
          value={origin}
          onChange={(e) => setOrigin(e.target.value)}
          placeholder="https://github.com/org/repo@0123456789abcdef0123456789abcdef01234567"
          className="w-full px-3 py-2 border border-gray-300 rounded-md text-sm font-mono focus:outline-none focus:ring-2 focus:ring-blue-500"
        />
        <p className="text-xs text-gray-500 mt-1">This field checks syntax only. If present, public provenance evidence must bind this exact Tree CID—not merely the same owner, domain, or repository.</p>
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
