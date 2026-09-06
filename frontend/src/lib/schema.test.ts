import { describe, expect, it } from "vitest";
import { canonicalRuntimes, getItemValidationError, runtimesValidationError } from "./schema";

describe("runtimes grammar (listing policy column 4)", () => {
  it("accepts well-formed lists", () => {
    for (const ok of ["generic", "claude_code", "claude_code,cursor", "a,b", "codex,gemini_cli,openclaw", "x1_2"]) {
      expect(runtimesValidationError(ok), ok).toBeNull();
    }
    expect(runtimesValidationError(Array.from({ length: 16 }, (_, i) => `r${String(i).padStart(2, "0")}`).join(","))).toBeNull();
  });
  it("rejects case, whitespace, hyphens, order, duplicates, generic mixing and size", () => {
    for (const bad of [
      "", "Generic", "claude code", "claude-code", "claude_code,", ",cursor", "cursor,claude_code",
      "cursor,cursor", "generic,cursor", "cursor,generic", "1abc", "_abc", "claude_code, cursor",
      "a".repeat(33),
    ]) {
      expect(runtimesValidationError(bad), bad).not.toBeNull();
    }
    expect(runtimesValidationError(Array.from({ length: 17 }, (_, i) => `r${String(i).padStart(2, "0")}`).join(","))).not.toBeNull();
  });
  it("canonicalizes to a sorted, deduplicated list", () => {
    expect(canonicalRuntimes(["cursor", "claude_code", "cursor"])).toBe("claude_code,cursor");
  });
  it("surfaces the runtimes error through the descriptor validator", () => {
    const fields = {
      name: "sample-skill",
      description: "A sample.",
      treeCid: "bafybeidgtfsc2ro3pfmyggmbz4ea7xg7g4gpehqur7klaadtreyjz6s3fu",
      runtimes: "cursor,claude_code",
      origin: "",
      reserved: "",
    };
    expect(getItemValidationError(fields)).toMatch(/ascending order/);
    expect(getItemValidationError({ ...fields, runtimes: "claude_code,cursor" })).toBeNull();
  });
});
