// SKILL.md frontmatter binding, with the same rules as the `intend` CLI's
// `policy.rs`: the document must START with a `---` line, the block ends at the
// next `---` line (CRLF tolerated), a real YAML parser reads it, duplicate keys
// are rejected, `name` and `description` must be scalars, unknown keys are
// ignored. Scalars are read under the failsafe schema so that a name such as
// `123` stays the string the descriptor holds.
import { parse } from "yaml";

export interface Frontmatter {
  name: string;
  description: string;
}

const isDashLine = (line: string): boolean => line.replace(/[\r\n]+$/, "") === "---";

export function frontmatterBlock(content: string): string {
  // Split keeping each line's newline; no lookbehind, which older WebKit lacks.
  const lines = content.split("\n").map((line, index, all) => (index < all.length - 1 ? `${line}\n` : line));
  if (lines.length > 0 && lines[lines.length - 1] === "") lines.pop();
  if (lines.length === 0 || !isDashLine(lines[0])) {
    throw new Error("SKILL.md does not start with a `---` frontmatter block");
  }
  const body: string[] = [];
  for (const line of lines.slice(1)) {
    if (isDashLine(line)) return body.join("");
    body.push(line);
  }
  throw new Error("SKILL.md frontmatter block is unterminated");
}

export function parseFrontmatter(content: string): Frontmatter {
  const block = frontmatterBlock(content);
  let parsed: unknown;
  try {
    parsed = parse(block, { schema: "failsafe", uniqueKeys: true });
  } catch (caught) {
    throw new Error(`SKILL.md frontmatter YAML: ${caught instanceof Error ? caught.message : String(caught)}`);
  }
  if (parsed === null || typeof parsed !== "object" || Array.isArray(parsed)) {
    throw new Error("SKILL.md frontmatter is not a mapping");
  }
  const record = parsed as Record<string, unknown>;
  const name = record.name;
  const description = record.description;
  if (typeof name !== "string") throw new Error("SKILL.md frontmatter `name` is missing or not a string");
  if (typeof description !== "string") throw new Error("SKILL.md frontmatter `description` is missing or not a string");
  return { name, description };
}
