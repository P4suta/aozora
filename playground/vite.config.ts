import { defineConfig, type Plugin } from 'vite';
import solidPlugin from 'vite-plugin-solid';
import { fileURLToPath } from 'node:url';

// Strict Content-Security-Policy for the production bundle. This is
// defense-in-depth layered *on top of* the renderer's escaping: every
// preview is mounted via `innerHTML` (HtmlPreview.tsx), but
// `aozora-render` already entity-escapes all text and never emits
// `<script>` / `on*=` / external `href`, so the CSP is a belt to the
// renderer's braces — a second wall if a future renderer regression ever
// let active markup through.
//
// Directive rationale (kept as tight as the app allows):
//   default-src 'self'            — same-origin baseline for everything.
//   script-src 'self'             — our bundle only…
//     'wasm-unsafe-eval'          — …plus WebAssembly.instantiate for the
//                                   parser wasm (no JS eval / unsafe-eval).
//   style-src 'self'              — hashed CSS assets…
//     'unsafe-inline'             — …plus the runtime <style> tags Solid
//                                   and CodeMirror inject (no nonce path).
//   img-src 'self' data:          — favicon + inline data: URIs.
//   font-src 'self'               — no external/CDN fonts are loaded.
//   connect-src 'self'            — covers the same-origin fetch() that
//                                   instantiateStreaming() uses to pull
//                                   `aozora_wasm_bg.wasm` (no data:/blob:).
//   object-src 'none'             — no <object>/<embed>/<applet>.
//   base-uri 'self'               — block <base> tag hijacking.
//   frame-ancestors 'none'        — disallow embedding (clickjacking).
// GitHub-issue navigations are <a target="_blank"> link clicks, which are
// navigations (not subresource loads) and need no allowlist here.
const PROD_CSP = [
  "default-src 'self'",
  "script-src 'self' 'wasm-unsafe-eval'",
  "style-src 'self' 'unsafe-inline'",
  "img-src 'self' data:",
  "font-src 'self'",
  "connect-src 'self'",
  "object-src 'none'",
  "base-uri 'self'",
  "frame-ancestors 'none'",
].join('; ');

// Inject the Content-Security-Policy meta tag only into the production
// build. In dev mode Vite needs an HMR WebSocket back to localhost which
// a strict `connect-src 'self'` would block; a `<meta>` CSP cannot be
// relaxed per-environment, so it is emitted at build time only.
function cspInProd(): Plugin {
  return {
    name: 'csp-in-prod',
    apply: 'build',
    transformIndexHtml: {
      order: 'pre',
      handler(html) {
        return html.replace(
          '<head>',
          `<head>\n    <meta http-equiv="Content-Security-Policy" content="${PROD_CSP}">`,
        );
      },
    },
  };
}

export default defineConfig({
  base: '/aozora/playground/',
  plugins: [solidPlugin(), cspInProd()],
  build: {
    rollupOptions: {
      // Two SPA entries, one per HTML file (no client router): the editor
      // playground (index.html → src/main.tsx) and the notation gallery
      // (gallery.html → src/gallery.tsx). Each is a Rollup input so `vite
      // build` emits both pages under the `base` path.
      input: {
        main: fileURLToPath(new URL('./index.html', import.meta.url)),
        gallery: fileURLToPath(new URL('./gallery.html', import.meta.url)),
      },
    },
  },
  resolve: {
    alias: {
      'aozora-wasm': fileURLToPath(
        new URL('../crates/aozora-wasm/pkg/aozora_wasm.js', import.meta.url),
      ),
    },
  },
  server: {
    fs: {
      allow: ['..'],
    },
  },
});
