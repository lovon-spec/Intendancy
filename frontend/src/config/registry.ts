export const REGISTRY_ADDRESS = (import.meta.env.VITE_REGISTRY_ADDRESS ||
  "0x0000000000000000000000000000000000000000") as `0x${string}`;

export const GUARDIAN_ADDRESS = (import.meta.env.VITE_GUARDIAN_ADDRESS ||
  "0x0000000000000000000000000000000000000000") as `0x${string}`;

export const COLUMNS = [
  { label: "Name", type: "text" },
  { label: "Source Type", type: "text" },
  { label: "Source Locator", type: "text" },
  { label: "Category", type: "text" },
  { label: "Runtimes", type: "text" },
  { label: "Description", type: "long text" },
] as const;

export const CATEGORIES = ["skill", "plugin", "convention"] as const;

export const RUNTIMES = [
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
