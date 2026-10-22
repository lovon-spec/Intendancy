import { ContractFunctionRevertedError, HttpRequestError, type Address } from "viem";
import { describe, expect, it } from "vitest";
import { findRequest, resolveDispute, type ContractReader, type RequestFields } from "./dispute";

const REGISTRY = "0x67DBE6A9597635074546e08B92eE617bF02168f9";
const COURT = "0x9C1dA9A04925bDfDedf0f6421bC7EEa8305F9002";
const WRAPPER = "0x1111111111111111111111111111111111111111";
const ITEM = "0xa5fc5c83dcd18cd740aa5a4aaa578c011528a8720444ff3ae5af5ddcc5ccaa72";

type Handler = (...args: never[]) => unknown;

/** A chain where each `address.function` answers from the table and anything else reverts, as a registry does for HOST(). */
function chain(table: Record<string, Handler>, calls: string[] = []): ContractReader {
  return {
    readContract: (async ({ address, functionName, args }: { address: Address; functionName: string; args?: readonly unknown[] }) => {
      const key = `${address.toLowerCase()}.${functionName}`;
      calls.push(`${key}(${(args ?? []).join(",")})`);
      const handler = table[key];
      if (!handler) throw new ContractFunctionRevertedError({ abi: [], functionName });
      return handler(...((args ?? []) as never[]));
    }) as unknown as ContractReader["readContract"],
  };
}

const at = (address: string, fn: string) => `${address.toLowerCase()}.${fn}`;

function wrapperChain(overrides: Record<string, Handler> = {}, calls?: string[]) {
  return chain(
    {
      [at(WRAPPER, "HOST")]: () => REGISTRY,
      [at(WRAPPER, "K")]: () => COURT,
      [at(WRAPPER, "localDispute")]: (remote: bigint) => {
        if (remote !== 1840n) throw new ContractFunctionRevertedError({ abi: [], functionName: "localDispute" });
        return 3n;
      },
      [at(REGISTRY, "arbitratorDisputeIDToItem")]: (arbitrator: string, id: bigint) =>
        arbitrator === WRAPPER && id === 3n ? ITEM : `0x${"0".repeat(64)}`,
      [at(WRAPPER, "caseOf")]: () => ({ kDisputeID: 1840n, binding: { item: ITEM, requestIndex: 2n } }),
      ...overrides,
    },
    calls,
  );
}

describe("dispute to item", () => {
  it("maps a dispute the registry raised directly, as before", async () => {
    const calls: string[] = [];
    const reader = chain({ [at(REGISTRY, "arbitratorDisputeIDToItem")]: (arbitrator: string, id: bigint) => (arbitrator === COURT && id === 42n ? ITEM : `0x${"0".repeat(64)}`) }, calls);
    const target = await resolveDispute(reader, { arbitrableContractAddress: REGISTRY, arbitratorContractAddress: COURT, disputeID: "42" });
    expect(target).toEqual({ registry: REGISTRY, itemID: ITEM, registryDispute: { arbitrator: COURT, disputeID: 42n } });
    expect(calls).toEqual([`${at(REGISTRY, "HOST")}()`, `${at(REGISTRY, "arbitratorDisputeIDToItem")}(${COURT},42)`]);
  });

  it("maps the court's dispute through the wrapper: remote ID, local case, host registry, item", async () => {
    const target = await resolveDispute(wrapperChain(), { arbitrableContractAddress: WRAPPER, arbitratorContractAddress: COURT, disputeID: "1840" });
    expect(target).toEqual({
      registry: REGISTRY,
      itemID: ITEM,
      requestIndex: 2,
      registryDispute: { arbitrator: WRAPPER, disputeID: 3n },
      viaWrapper: { wrapper: WRAPPER, court: COURT, remoteID: 1840n, localID: 3n },
    });
  });

  it("refuses a wrapper route that does not hold together", async () => {
    const params = { arbitrableContractAddress: WRAPPER, arbitratorContractAddress: COURT, disputeID: "1840" } as const;
    await expect(resolveDispute(wrapperChain({ [at(WRAPPER, "K")]: () => REGISTRY }), params)).rejects.toThrow(/forwards to 0x67DB…68f9, not to the arbitrator 0x9C1d…9002/);
    await expect(resolveDispute(wrapperChain(), { ...params, disputeID: "7" })).rejects.toThrow("Dispute 7 on 0x9C1d…9002 was not forwarded by the wrapper 0x1111…1111");
    await expect(resolveDispute(wrapperChain({ [at(WRAPPER, "caseOf")]: () => ({ kDisputeID: 1841n, binding: { item: ITEM, requestIndex: 2n } }) }), params)).rejects.toThrow(/do not match/);
    // An RPC failure on HOST() is not a registry answering "no such function".
    const down = { readContract: async () => { throw new HttpRequestError({ url: "http://127.0.0.1:9" }); } } as unknown as ContractReader;
    await expect(resolveDispute(down, params)).rejects.toThrow(/HTTP request failed/);
  });

  it("finds the request a dispute belongs to after the item moved on", async () => {
    const request = (disputeID: bigint, arbitrator: Address, disputed = true): RequestFields => ({
      disputed, disputeID, arbitrator, submissionTime: 0n, resolved: true, requester: COURT, challenger: COURT, ruling: 1,
    });
    const requests = [request(9n, COURT), request(42n, COURT), request(0n, COURT, false)];
    const read = async (index: number) => requests[index];
    expect((await findRequest(3, read, { arbitrator: COURT, disputeID: 42n })).index).toBe(1);
    expect((await findRequest(3, read, { arbitrator: WRAPPER, disputeID: 42n })).index).toBe(2);
    expect((await findRequest(3, read)).index).toBe(2);
    expect((await findRequest(3, read, { arbitrator: COURT, disputeID: 9n }, 0)).index).toBe(0);
  });
});
