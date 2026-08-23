import { toRlp, fromRlp, stringToHex, toBytes, keccak256, type Hex } from "viem";
import type { ItemFields } from "../types";
import { getItemValidationError } from "./schema";

const COLUMN_ORDER: (keyof ItemFields)[] = [
  "name",
  "description",
  "treeCid",
  "runtimes",
  "origin",
  "reserved",
];

/** RLP-encode an item's fields into the bytes expected by GeneralizedTCR.addItem() */
export function encodeItem(fields: ItemFields): Hex {
  const validationError = getItemValidationError(fields);
  if (validationError) throw new Error(validationError);
  const values: Hex[] = COLUMN_ORDER.map((key) => stringToHex(fields[key]));
  return toRlp(values);
}

/** Decode RLP-encoded item bytes back into fields */
export function decodeItem(data: Hex): ItemFields {
  const decoded = fromRlp(data, "hex");
  if (!Array.isArray(decoded) || decoded.length !== 6) {
    throw new Error(`Expected 6 RLP fields, got ${Array.isArray(decoded) ? decoded.length : "non-array"}`);
  }
  if (!decoded.every((value) => typeof value === "string")) {
    throw new Error("Expected every RLP field to be a byte string");
  }
  const encodedFields = decoded as Hex[];
  if (toRlp(encodedFields).toLowerCase() !== data.toLowerCase()) {
    throw new Error("Descriptor is not canonical RLP or contains trailing bytes");
  }
  const strings = encodedFields.map((hex) => {
    const bytes = toBytes(hex as Hex);
    return new TextDecoder("utf-8", { fatal: true }).decode(bytes);
  });
  return {
    name: strings[0],
    description: strings[1],
    treeCid: strings[2],
    runtimes: strings[3],
    origin: strings[4],
    reserved: strings[5],
  };
}

/** Compute the item ID (keccak256 of encoded bytes) */
export function computeItemId(fields: ItemFields): Hex {
  return keccak256(encodeItem(fields));
}
