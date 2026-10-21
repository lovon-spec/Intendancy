import { describe, expect, it } from "vitest";
import { readParams } from "./params";

const REGISTRY = "0x54A92C21C9Ab4d76C2a15c3d9F71F2c8a5E56610";
const ARBITRATOR = "0x9C1dA9A04925bDfDedf0f6421bC7EEa8305F9002";

describe("display parameters", () => {
  it("reads the court's URL-encoded JSON form", () => {
    const search = "?" + encodeURIComponent(JSON.stringify({ disputeID: "42", arbitrableContractAddress: REGISTRY, arbitratorContractAddress: ARBITRATOR, arbitrableChainID: "100", arbitrableJsonRpcUrl: "https://rpc.gnosischain.com" }));
    expect(readParams(search)).toEqual({
      disputeID: "42",
      arbitrableContractAddress: REGISTRY,
      arbitratorContractAddress: ARBITRATOR,
      arbitrableChainID: 100,
      arbitrableJsonRpcUrl: "https://rpc.gnosischain.com",
    });
  });
  it("reads the legacy partially escaped JSON form", () => {
    const search = `?%7B%22disputeID%22%3A%227%22%2C%22arbitrableContractAddress%22%3A%22${REGISTRY}%22%2C%22arbitratorContractAddress%22%3A%22${ARBITRATOR}%22%2C%22arbitrableChainID%22%3A100%7D`;
    const params = readParams(search);
    expect(params.disputeID).toBe("7");
    expect(params.arbitrableChainID).toBe(100);
    expect(params.arbitrableContractAddress).toBe(REGISTRY);
  });
  it("reads the court's query-string form, which MetaEvidence _v 1.0.0 selects", () => {
    // Built the way kleros/court case-details-card.jsx builds it when `_v` is not "0".
    const injected = {
      disputeID: "1840",
      chainID: "100",
      arbitratorContractAddress: ARBITRATOR,
      arbitratorJsonRpcUrl: "https://rpc.example/gnosis",
      arbitratorChainID: "100",
      arbitrableContractAddress: REGISTRY,
      arbitrableChainID: "100",
      arbitrableJsonRpcUrl: "https://rpc.example/gnosis",
    };
    expect(readParams(`?${new URLSearchParams(injected).toString()}`)).toEqual({
      disputeID: "1840",
      arbitrableContractAddress: REGISTRY,
      arbitratorContractAddress: ARBITRATOR,
      arbitrableChainID: 100,
      arbitrableJsonRpcUrl: "https://rpc.example/gnosis",
      itemID: undefined,
    });
  });
  it("reads the plain form and drops malformed values", () => {
    const item = `0x${"ab".repeat(32)}`;
    expect(readParams(`?registry=${REGISTRY}&item=${item}&chain=31337`)).toEqual({
      arbitrableContractAddress: REGISTRY,
      itemID: item,
      disputeID: undefined,
      arbitratorContractAddress: undefined,
      arbitrableChainID: 31337,
      arbitrableJsonRpcUrl: undefined,
    });
    expect(readParams("?registry=nope&item=0x12&chain=-1")).toEqual({
      arbitrableContractAddress: undefined,
      itemID: undefined,
      disputeID: undefined,
      arbitratorContractAddress: undefined,
      arbitrableChainID: undefined,
      arbitrableJsonRpcUrl: undefined,
    });
  });
  it("returns nothing for an empty query", () => {
    expect(readParams("")).toEqual({});
    expect(readParams("?")).toEqual({});
  });
});
