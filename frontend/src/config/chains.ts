import { defineChain } from "viem";
import { gnosis } from "viem/chains";

export { gnosis };

export const localFork = defineChain({
  id: 31337,
  name: "Anvil Fork",
  nativeCurrency: { name: "xDAI", symbol: "xDAI", decimals: 18 },
  rpcUrls: { default: { http: ["http://127.0.0.1:8545"] } },
});

const chainId = Number(import.meta.env.VITE_CHAIN_ID || "100");
export const activeChain = chainId === 31337 ? localFork : gnosis;
