import type { ItemFields } from "../types";

const NAME_PATTERN = /^[a-z0-9]+(?:-[a-z0-9]+)*$/;
const BASE32_PATTERN = /^b[a-z2-7]+$/;
const BASE32_ALPHABET = "abcdefghijklmnopqrstuvwxyz234567";
const RUNTIME_IDENTIFIER = /^[a-z][a-z0-9_]{0,31}$/;
export const MAX_RUNTIMES = 16;

/** Listing policy column 4: lowercase snake_case identifiers, single commas, unique, ascending, at most 16, `generic` only alone. */
export function runtimesValidationError(value: string): string | null {
  if (value === "") return "Runtimes must list at least one identifier";
  const parts = value.split(",");
  if (parts.length > MAX_RUNTIMES) return `Runtimes lists ${parts.length} identifiers, at most ${MAX_RUNTIMES} allowed`;
  for (let i = 0; i < parts.length; i += 1) {
    if (!RUNTIME_IDENTIFIER.test(parts[i])) {
      return `Runtimes identifier "${parts[i]}" is not lowercase snake case (a letter, then letters, digits or underscores, at most 32 characters)`;
    }
    if (i > 0 && parts[i - 1] >= parts[i]) {
      return `Runtimes identifiers must be unique and in ascending order ("${parts[i - 1]}" before "${parts[i]}")`;
    }
  }
  if (parts.length > 1 && parts.includes("generic")) return "Runtimes: generic must be the only identifier when present";
  return null;
}

/** Canonical form of a set of runtime identifiers: sorted, deduplicated, joined with single commas. */
export function canonicalRuntimes(identifiers: string[]): string {
  return [...new Set(identifiers)].sort().join(",");
}

function decodeBase32(value: string): Uint8Array | null {
  let buffer = 0;
  let bits = 0;
  const output: number[] = [];
  for (const character of value) {
    const digit = BASE32_ALPHABET.indexOf(character);
    if (digit < 0) return null;
    buffer = (buffer << 5) | digit;
    bits += 5;
    if (bits >= 8) {
      bits -= 8;
      output.push((buffer >> bits) & 0xff);
      buffer &= (1 << bits) - 1;
    }
  }
  // Canonical unpadded base32 has only zero-valued unused trailing bits.
  if (buffer !== 0) return null;
  return Uint8Array.from(output);
}

function encodeBase32(bytes: Uint8Array): string {
  let buffer = 0;
  let bits = 0;
  let output = "";
  for (const byte of bytes) {
    buffer = (buffer << 8) | byte;
    bits += 8;
    while (bits >= 5) {
      bits -= 5;
      output += BASE32_ALPHABET[(buffer >> bits) & 31];
      buffer &= (1 << bits) - 1;
    }
  }
  if (bits > 0) output += BASE32_ALPHABET[(buffer << (5 - bits)) & 31];
  return output;
}

function readVarint(bytes: Uint8Array, start: number): [number, number] | null {
  let value = 0;
  let multiplier = 1;
  for (let index = start; index < bytes.length; index += 1) {
    const byte = bytes[index];
    value += (byte & 0x7f) * multiplier;
    if (!Number.isSafeInteger(value)) return null;
    if ((byte & 0x80) === 0) {
      // Unsigned varints must use their shortest representation.
      if (index > start && (byte & 0x7f) === 0) return null;
      return [value, index + 1];
    }
    multiplier *= 128;
  }
  return null;
}

function isHttpUrl(value: string): boolean {
  try {
    const url = new URL(value);
    return (
      (url.protocol === "https:" || url.protocol === "http:") &&
      url.username === "" &&
      url.password === ""
    );
  } catch {
    return false;
  }
}

function splitPinnedGitOrigin(value: string): { repository: string; commit: string } | null {
  const at = value.lastIndexOf("@");
  if (at < 0) return null;
  const repository = value.slice(0, at);
  const commit = value.slice(at + 1);
  if (!/^[0-9a-f]{40}$/i.test(commit) || !isHttpUrl(repository)) return null;
  return { repository, commit };
}

export function isCanonicalTreeCid(value: string): boolean {
  if (!BASE32_PATTERN.test(value)) return false;
  const bytes = decodeBase32(value.slice(1));
  if (!bytes) return false;
  if (`b${encodeBase32(bytes)}` !== value) return false;

  const version = readVarint(bytes, 0);
  if (!version || version[0] !== 1) return false;
  const codec = readVarint(bytes, version[1]);
  // UnixFS directories use the dag-pb multicodec (0x70).
  if (!codec || codec[0] !== 0x70) return false;
  const hashCode = readVarint(bytes, codec[1]);
  // V1 deliberately allows only sha2-256 (multihash code 0x12).
  if (!hashCode || hashCode[0] !== 0x12) return false;
  const digestLength = readVarint(bytes, hashCode[1]);
  return Boolean(
    digestLength &&
    digestLength[0] === 32 &&
    digestLength[1] + digestLength[0] === bytes.length,
  );
}

export function isValidOrigin(value: string): boolean {
  return value === "" || splitPinnedGitOrigin(value) !== null || isHttpUrl(value);
}

export function originHref(value: string): string | null {
  const pinnedGit = splitPinnedGitOrigin(value);
  if (pinnedGit) {
    const { repository, commit } = pinnedGit;
    if (repository.includes("github.com") || repository.includes("gitlab.com")) {
      return `${repository}/tree/${commit}`;
    }
    return repository;
  }
  return isHttpUrl(value) ? value : null;
}

/** Return the first V1 descriptor-profile violation, suitable for form feedback. */
export function getItemValidationError(fields: ItemFields): string | null {
  if (fields.name.length < 1 || fields.name.length > 64 || !NAME_PATTERN.test(fields.name)) {
    return "Name must be 1–64 lowercase letters, numbers, or single hyphens, with no leading, trailing, or consecutive hyphens";
  }
  if (fields.description.length < 1 || [...fields.description].length > 1024) {
    return "Description must be between 1 and 1024 characters";
  }
  if (!isCanonicalTreeCid(fields.treeCid)) {
    return "Tree CID must be a canonical bare lowercase CIDv1 base32 dag-pb CID with a 32-byte SHA-256 multihash (for example, bafy...); the root is checked as a UnixFS directory when fetched";
  }
  const runtimesError = runtimesValidationError(fields.runtimes);
  if (runtimesError) return runtimesError;
  if (!isValidOrigin(fields.origin)) {
    return "Origin must be empty, an HTTP(S) publisher URL, or a public git URL followed by @ and a full 40-character commit hash";
  }
  if (fields.reserved !== "") {
    return "Reserved must be empty under the current descriptor profile";
  }
  return null;
}
