import { Link } from "react-router-dom";
import Markdown from "react-markdown";

const POLICY = `# Intendhub Registry — Listing Policy

**Version**: 1.0
**Registry**: Intendhub Agent Skills, Plugins & Conventions Registry
**Chain**: Gnosis Chain

## Purpose

This registry curates AI agent skills, plugins, and convention files that have been verified by the community as non-malicious, accurately described, and functional. Entries are verified through Kleros dispute resolution.

## Entry Types

- **skill** — A SKILL.md file (or directory) following the Agent Skills specification
- **plugin** — A bundle of skills, hooks, MCP server configurations, and/or slash commands
- **convention** — An AGENTS.md, CLAUDE.md, .cursorrules, or similar agent instruction file

## Required Fields

Each submission must include all of the following:

1. **Name** — A human-readable name for the entry
2. **Source Type** — Must be \`git\`
3. **Source Locator** — Any public git repository URL followed by \`@\` and the full 40-character SHA-1 commit hash (e.g., \`https://github.com/org/repo@a1b2c3d4e5f6a1b2c3d4e5f6a1b2c3d4e5f6a1b2\`)
4. **Category** — One of: \`skill\`, \`plugin\`, \`convention\`
5. **Runtimes** — Comma-separated list of supported agent runtimes (e.g., \`claude_code,cursor,generic\`). Use \`generic\` if the entry follows the cross-runtime Agent Skills specification without runtime-specific features.
6. **Description** — A brief, accurate description of what the entry does

## Acceptance Criteria

An entry MUST satisfy ALL of the following criteria to be accepted:

### 1. Accurate Description
The entry's Name, Category, Runtimes, and Description must accurately represent its actual content and behavior. The Description must not be misleading or deceptive.

### 2. Valid Source
The Source Locator must point to a publicly accessible git repository. The commit hash must exist in the repository. The content at the specified commit must be retrievable by anyone.

### 3. Correct Category
The Category must accurately reflect the entry type:
- \`skill\` — Must contain a valid SKILL.md file with YAML frontmatter
- \`plugin\` — Must contain a plugin manifest or bundle structure
- \`convention\` — Must contain an agent instruction/convention file

### 4. Declared Runtimes
The entry must actually work (or be designed to work) with each of the declared runtimes. Runtimes must not be listed speculatively.

### 5. No Malicious Behavior
The entry must NOT:
- Exfiltrate user data, credentials, or environment variables
- Contain prompt injection attacks in tool descriptions or instructions
- Download or execute undeclared external binaries
- Disable security mechanisms or sandboxing
- Install persistent backdoors
- Perform cryptocurrency mining or resource abuse

### 6. No Impersonation
The entry must not impersonate, typosquat, or misleadingly copy another project's name, description, or branding.

### 7. No Duplicates
The entry must not be a duplicate of an already-registered entry at the same source locator and commit hash.

## Removal Criteria

A registered entry should be removed if:
- It no longer satisfies any of the acceptance criteria above
- The source repository has been deleted or made private (source is no longer publicly accessible)
- A security vulnerability has been discovered in the entry

## Guardian Convention

A designated guardian address may submit standard removal requests to flag potentially dangerous entries. When the guardian submits a removal request, downstream consumers (frontends, CLIs, APIs) SHOULD treat the entry as immediately flagged for safety review, even before the challenge period completes.

The guardian has no special on-chain privileges. It submits standard removal requests and posts standard removal deposits. If the guardian's removal is unjustified, anyone can challenge it and win the guardian's deposit through normal Kleros arbitration.

## Evidence Guidelines

When challenging a submission or removal, challengers should provide:
- A clear explanation of which acceptance criterion the entry violates
- Links to specific files or code in the source repository that demonstrate the violation
- Screenshots or logs if the violation is behavioral (e.g., network requests to external servers)
- References to automated security scan results if available

## Juror Instructions

When evaluating a dispute:
1. Read this listing policy carefully
2. Review the submitted evidence from both parties
3. Check whether the entry meets ALL acceptance criteria
4. If the entry fails ANY criterion, rule to reject it (for registration disputes) or rule to remove it (for removal disputes)
5. If the entry meets all criteria, rule to accept it (for registration disputes) or rule to keep it (for removal disputes)
6. When in doubt, prefer the interpretation that protects end users from potential harm
`;

export function PolicyPage() {
  return (
    <div className="max-w-3xl mx-auto">
      <Link to="/" className="text-sm text-blue-600 hover:underline">&larr; Back to registry</Link>
      <div className="mt-4 prose prose-sm prose-gray max-w-none [&_h1]:text-2xl [&_h1]:font-bold [&_h1]:mb-4 [&_h2]:text-lg [&_h2]:font-semibold [&_h2]:mt-6 [&_h2]:mb-2 [&_h3]:text-base [&_h3]:font-semibold [&_h3]:mt-4 [&_h3]:mb-1 [&_p]:text-sm [&_p]:text-gray-700 [&_p]:mb-3 [&_li]:text-sm [&_li]:text-gray-700 [&_ul]:mb-3 [&_ol]:mb-3 [&_code]:text-xs [&_code]:bg-gray-100 [&_code]:px-1 [&_code]:py-0.5 [&_code]:rounded [&_strong]:text-gray-900">
        <Markdown>{POLICY}</Markdown>
      </div>
    </div>
  );
}
