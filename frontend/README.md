# Intendancy frontend

React/Vite interface for browsing, inspecting, and submitting skill entries to the
Intendancy Kleros Curate registry.

## Local setup

```bash
npm ci
cp .env.example .env
npm run dev
```

Configure the local `.env` with:

- `VITE_REGISTRY_ADDRESS`: deployed registry contract address.
- `VITE_CHAIN_ID`: target chain ID (`100` for Gnosis).
- `VITE_RPC_URL`: JSON-RPC endpoint used for registry reads.

Local environment files are ignored. Keep `.env.example` limited to public defaults and
place credentials only in `.env`.

## Checks

```bash
npm run lint
npm run build
```

## Evidence display

The juror-facing page the court loads for disputed requests lives in `evidence-display/` and builds separately with `npm run build:evidence`; see [`evidence-display/README.md`](evidence-display/README.md). `npm test` runs the in-browser tree verifier against the CLI's kubo interop vectors.
