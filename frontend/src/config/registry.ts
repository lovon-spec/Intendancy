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

export const SOURCE_TYPES = ["git", "npm", "pypi", "docker", "ipfs"] as const;

export type SourceType = (typeof SOURCE_TYPES)[number];

export interface SourceTypeConfig {
  label: string;
  placeholder: string;
  helpText: string;
  validate: (locator: string) => boolean;
  toUrl: (locator: string) => string | null;
}

export const SOURCE_TYPE_CONFIG: Record<SourceType, SourceTypeConfig> = {
  git: {
    label: "Git Repository",
    placeholder: "https://github.com/org/repo@a1b2c3d4e5f6a7b8c9d0e1f2a3b4c5d6e7f8a9b0",
    helpText: "Any public git repository URL followed by @ and the full 40-character SHA-1 commit hash",
    validate: (l) => /^https?:\/\/.+@[0-9a-f]{40}$/i.test(l),
    toUrl: (l) => {
      const at = l.lastIndexOf("@");
      if (at === -1) return null;
      const repo = l.slice(0, at);
      const hash = l.slice(at + 1);
      // GitHub/GitLab: link to the commit
      if (repo.includes("github.com") || repo.includes("gitlab.com")) {
        return `${repo}/tree/${hash}`;
      }
      return repo;
    },
  },
  npm: {
    label: "npm Package",
    placeholder: "@anthropic/skills@1.2.3#sha512-abc123...",
    helpText: "Package name@version, optionally followed by #integrity hash (from npm view <pkg> dist.integrity)",
    validate: (l) => /^(@[a-z0-9-~][a-z0-9-._~]*\/)?[a-z0-9-~][a-z0-9-._~]*@\d+\.\d+\.\d+/.test(l),
    toUrl: (l) => {
      const at = l.indexOf("@", l.startsWith("@") ? 1 : 0);
      const hashIdx = l.indexOf("#");
      const name = l.slice(0, at);
      const version = hashIdx > -1 ? l.slice(at + 1, hashIdx) : l.slice(at + 1);
      return `https://www.npmjs.com/package/${name}/v/${version}`;
    },
  },
  pypi: {
    label: "PyPI Package",
    placeholder: "mcp-server-github==0.3.1#sha256:abc123...",
    helpText: "Package==version, optionally followed by #sha256:digest (from PyPI JSON API)",
    validate: (l) => /^[a-zA-Z0-9_-]+==\d+\.\d+/.test(l),
    toUrl: (l) => {
      const eqIdx = l.indexOf("==");
      const hashIdx = l.indexOf("#");
      const name = l.slice(0, eqIdx);
      const version = hashIdx > -1 ? l.slice(eqIdx + 2, hashIdx) : l.slice(eqIdx + 2);
      return `https://pypi.org/project/${name}/${version}/`;
    },
  },
  docker: {
    label: "Docker Image",
    placeholder: "mcp/postgres@sha256:a1b2c3d4e5f6...",
    helpText: "Image name@sha256:digest (immutable digest, not a tag)",
    validate: (l) => /^[a-z0-9._/-]+@sha256:[0-9a-f]{64}$/i.test(l),
    toUrl: (l) => {
      const at = l.indexOf("@");
      const image = l.slice(0, at);
      // Docker Hub
      if (!image.includes(".")) {
        const parts = image.split("/");
        const ns = parts.length > 1 ? parts[0] : "library";
        const name = parts.length > 1 ? parts[1] : parts[0];
        return `https://hub.docker.com/r/${ns}/${name}`;
      }
      return null;
    },
  },
  ipfs: {
    label: "IPFS CID",
    placeholder: "bafybeigdyrzt5sfp7udm7hu76uh7y26nf3efuylqabf3oclgtqy55fbzdi",
    helpText: "IPFS Content Identifier (CIDv1 or CIDv0). Already content-addressed.",
    validate: (l) => /^(bafy[a-z0-9]{50,}|Qm[a-zA-Z0-9]{44,})$/.test(l),
    toUrl: (l) => `https://dweb.link/ipfs/${l}`,
  },
};

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
