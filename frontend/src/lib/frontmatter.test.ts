import { describe, expect, it } from "vitest";
import { parseFrontmatter } from "./frontmatter";

describe("frontmatter binding, same rules as the CLI", () => {
  it("reads plain scalars", () => {
    expect(parseFrontmatter("---\nname: my-skill\ndescription: Does things.\n---\nBody.\n")).toEqual({
      name: "my-skill",
      description: "Does things.",
    });
  });
  it("reads quoted scalars and ignores comments", () => {
    expect(parseFrontmatter("---\nname: \"my-skill\" # comment\ndescription: 'Does things.'\n---\n")).toEqual({
      name: "my-skill",
      description: "Does things.",
    });
  });
  it("tolerates CRLF", () => {
    expect(parseFrontmatter("---\r\nname: a\r\ndescription: b\r\n---\r\n")).toEqual({ name: "a", description: "b" });
  });
  it("reads block scalars", () => {
    expect(parseFrontmatter("---\nname: a\ndescription: |-\n  Multi\n  line\n---\n").description).toBe("Multi\nline");
  });
  it("keeps a numeric-looking name as the string the descriptor holds", () => {
    expect(parseFrontmatter("---\nname: 123\ndescription: c\n---\n").name).toBe("123");
  });
  it("ignores unknown keys", () => {
    expect(parseFrontmatter("---\nname: a\nextra: 1\ndescription: c\n---\n")).toEqual({ name: "a", description: "c" });
  });
  it("rejects duplicate keys, sequences, missing fields and missing blocks", () => {
    expect(() => parseFrontmatter("---\nname: a\nname: b\ndescription: c\n---\n")).toThrow(/YAML/);
    expect(() => parseFrontmatter("---\nname: [1,2]\ndescription: c\n---\n")).toThrow(/not a string/);
    expect(() => parseFrontmatter("no frontmatter")).toThrow(/does not start/);
    expect(() => parseFrontmatter("---\nname: x\n")).toThrow(/unterminated/);
    expect(() => parseFrontmatter("---\ndescription: only\n---\n")).toThrow(/name/);
  });
});
