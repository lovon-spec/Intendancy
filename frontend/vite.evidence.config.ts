import { fileURLToPath } from "node:url";
import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";
import tailwindcss from "@tailwindcss/vite";

// The juror-facing evidence display: a self-contained static bundle with
// relative asset paths, built for pinning on IPFS and referenced from the
// registry's MetaEvidence as `evidenceDisplayInterfaceURI`.
export default defineConfig({
  root: "evidence-display",
  base: "./",
  publicDir: false,
  envDir: fileURLToPath(new URL(".", import.meta.url)),
  plugins: [react(), tailwindcss()],
  build: { outDir: "../dist-evidence", emptyOutDir: true },
});
