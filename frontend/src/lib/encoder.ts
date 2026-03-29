import { toRlp, fromRlp, toHex, toBytes, keccak256, type Hex } from "viem";
import type { ItemFields } from "../types";

const COLUMN_ORDER: (keyof ItemFields)[] = [
  "name",
  "sourceType",
  "sourceLocator",
  "category",
  "runtimes",
  "description",
];

/** RLP-encode an item's fields into the bytes expected by GeneralizedTCR.addItem() */
export function encodeItem(fields: ItemFields): Hex {
  const values: Hex[] = COLUMN_ORDER.map((key) => toHex(toBytes(fields[key])));
  return toRlp(values);
}

/** Decode RLP-encoded item bytes back into fields */
export function decodeItem(data: Hex): ItemFields {
  const decoded = fromRlp(data, "hex");
  if (!Array.isArray(decoded) || decoded.length !== 6) {
    throw new Error(`Expected 6 RLP fields, got ${Array.isArray(decoded) ? decoded.length : "non-array"}`);
  }
  const strings = decoded.map((hex) => {
    const bytes = toBytes(hex as Hex);
    return new TextDecoder().decode(bytes);
  });
  return {
    name: strings[0],
    sourceType: strings[1],
    sourceLocator: strings[2],
    category: strings[3] as ItemFields["category"],
    runtimes: strings[4],
    description: strings[5],
  };
}

/** Compute the item ID (keccak256 of encoded bytes) */
export function computeItemId(fields: ItemFields): Hex {
  return keccak256(encodeItem(fields));
}
