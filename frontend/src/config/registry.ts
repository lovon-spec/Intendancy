export const REGISTRY_ADDRESS = (import.meta.env.VITE_REGISTRY_ADDRESS ||
  "0x0000000000000000000000000000000000000000") as `0x${string}`;

export const COLUMNS = [
  { label: "Name", type: "text" },
  { label: "Description", type: "long text" },
  { label: "Tree CID", type: "text" },
  { label: "Runtimes", type: "text" },
  { label: "Origin", type: "text" },
  { label: "Reserved", type: "text" },
] as const;

export const SUGGESTED_RUNTIMES = [
  "claude_code",
  "openclaw",
  "cursor",
  "codex",
  "gemini_cli",
  "copilot",
  "windsurf",
  "aider",
  "roo_code",
  "intendant",
  "generic",
] as const;
