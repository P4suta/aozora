import { defineConfig, type Plugin } from 'vite';
import solidPlugin from 'vite-plugin-solid';
import { fileURLToPath } from 'node:url';

const PROD_CSP =
  "default-src 'self'; script-src 'self' 'wasm-unsafe-eval'; style-src 'self' 'unsafe-inline'; img-src 'self' data:;";

// Inject a Content-Security-Policy meta tag only into the
// production build. In dev mode Vite needs an HMR WebSocket back to
// localhost which a strict CSP `connect-src 'self'` would block.
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
