# Aozora Playground

The interactive web playground for 青空文庫記法:
<https://p4suta.github.io/aozora/playground/>

Solid + Vite + CodeMirror 6, over the parser compiled to WebAssembly.

## Local development

```sh
docker compose run --rm dev wasm-pack build --target web --release crates/aozora-wasm
docker compose run --rm playground bun install
docker compose up playground     # → http://localhost:5173/aozora/playground/
```

Vite aliases `aozora-wasm` to `../crates/aozora-wasm/pkg/`, so re-running
the first command and reloading the page picks up a Rust-side change.

`just playground-ci` is the gate.

## Build

`docker compose run --rm playground bun run build` → `playground/dist/`.

`vite.config.ts` injects a strict CSP into the production build only. A
`<meta>` CSP cannot be relaxed per-environment, and the dev HMR
WebSocket needs it absent.
