# Aozora Playground

The interactive web playground for 青空文庫記法:
<https://p4suta.github.io/aozora/playground/>

React 19 + Adobe Spectrum 2 + CodeMirror 6, over the parser compiled to
WebAssembly. The private `../playground-ui/` package is the canonical
application shell shared byte-for-byte with Aozora Flavored Markdown.

## Local development

```sh
./bootstrap
just playground-wasm
cd playground && bun run dev     # → http://localhost:5173/aozora/playground/
```

Vite aliases `aozora-wasm` to `../crates/aozora-wasm/pkg/`, so re-running
the first command and reloading the page picks up a Rust-side change.

`just playground-ci` is the unit, coverage, type, style, and static migration
gate. `just playground-e2e` exercises the production bundle and real WASM in
Chromium, Firefox, and WebKit. `just playground-lighthouse` enforces the
desktop and mobile performance and accessibility budgets for both
`index.html` and `gallery.html`.

## Build

`just playground-build` → `playground/dist/`.

`vite.config.ts` injects a strict CSP into the production build only. A
`<meta>` CSP cannot be relaxed per-environment, and the dev HMR
WebSocket needs it absent.
