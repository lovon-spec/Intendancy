import { BrowserRouter, Routes, Route } from "react-router-dom";
import { WagmiProvider } from "wagmi";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { config } from "./config/wagmi";
import { Layout } from "./components/layout/Layout";
import { RegistryPage } from "./pages/RegistryPage";
import { ItemPage } from "./pages/ItemPage";
import { SubmitPage } from "./pages/SubmitPage";
import { PolicyPage } from "./pages/PolicyPage";

const queryClient = new QueryClient({
  defaultOptions: {
    queries: {
      staleTime: 30_000,
      retry: 2,
    },
  },
});

export default function App() {
  return (
    <WagmiProvider config={config}>
      <QueryClientProvider client={queryClient}>
        <BrowserRouter basename={import.meta.env.BASE_URL}>
          <Routes>
            <Route element={<Layout />}>
              <Route path="/" element={<RegistryPage />} />
              <Route path="/item/:itemId" element={<ItemPage />} />
              <Route path="/submit" element={<SubmitPage />} />
              <Route path="/policy" element={<PolicyPage />} />
            </Route>
          </Routes>
        </BrowserRouter>
      </QueryClientProvider>
    </WagmiProvider>
  );
}
