import { http, createConfig } from "wagmi";
import { injected } from "wagmi/connectors";
import { activeChain, gnosis, localFork } from "./chains";

const rpcUrl = import.meta.env.VITE_RPC_URL;

export const config = createConfig({
  chains: [activeChain],
  connectors: [injected()],
  transports: {
    [gnosis.id]: http(rpcUrl || undefined),
    [localFork.id]: http("http://127.0.0.1:8545"),
  },
});
