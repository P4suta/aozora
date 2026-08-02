import { fileURLToPath } from 'node:url';
import optimizeLocales from '@react-aria/optimize-locales-plugin';
import react from '@vitejs/plugin-react';
import macros from 'unplugin-parcel-macros';
import type { Plugin } from 'vite';
import { defineConfig } from 'vitest/config';

// frame-ancestors is response-header-only; including it in a meta policy would
// claim clickjacking protection that GitHub Pages cannot actually enforce.
const PROD_CSP = [
  "default-src 'self'",
  "script-src 'self' 'wasm-unsafe-eval'",
  "style-src 'self' 'unsafe-inline'",
  "img-src 'self' data:",
  "font-src 'self'",
  "connect-src 'self'",
  "object-src 'none'",
  "base-uri 'self'",
].join('; ');

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

const OFFLINE_SPECTRUM_FONTS = '\0offline-spectrum-fonts';

function offlineSpectrumFonts(): Plugin {
  return {
    name: 'offline-spectrum-fonts',
    enforce: 'pre',
    resolveId(source, importer) {
      if (
        importer?.includes('@react-spectrum/s2/') &&
        /\/Provider\.(?:mjs|tsx)$/.test(importer) &&
        /^\.\/Fonts(?:\.mjs)?$/.test(source)
      ) {
        return OFFLINE_SPECTRUM_FONTS;
      }
      return null;
    },
    load(id) {
      return id === OFFLINE_SPECTRUM_FONTS
        ? 'export function Fonts() { return null; }'
        : null;
    },
  };
}

type MacroPlugin = Plugin & {
  transformInclude?: (id: string) => boolean;
};

function spectrumMacros(): Plugin {
  const plugin = macros.vite() as MacroPlugin;
  plugin.transformInclude = (id) =>
    /\.(?:js|jsx|ts|tsx)$/.test(id) &&
    !(id.includes('/playground-ui/src/') && !id.includes('/node_modules/')) &&
    (!id.includes('/node_modules/') ||
      id.includes('/node_modules/@aozora/playground-ui/'));
  return plugin;
}

const root = new URL('.', import.meta.url);
const wasm = new URL('../crates/aozora-wasm/pkg/aozora_wasm.js', root);
const wasmStub = new URL(
  './src/__tests__/__stubs__/aozora-wasm.ts',
  import.meta.url,
);
const spectrumStyleStub = new URL(
  './src/__tests__/__stubs__/s2-style.ts',
  import.meta.url,
);

export default defineConfig(({ command, isPreview, mode }) => ({
  base: command === 'build' || isPreview ? '/aozora/playground/' : '/',
  plugins: [
    offlineSpectrumFonts(),
    spectrumMacros(),
    react(),
    {
      ...optimizeLocales.vite({ locales: ['en', 'ja'] }),
      enforce: 'pre',
    },
    cspInProd(),
  ],
  resolve: {
    dedupe: [
      '@react-spectrum/s2',
      '@testing-library/react',
      '@testing-library/user-event',
      'lz-string',
      'react',
      'react-dom',
    ],
    preserveSymlinks: true,
    alias: [
      ...(mode === 'test'
        ? [
            {
              find: /^@react-spectrum\/s2\/style$/,
              replacement: fileURLToPath(spectrumStyleStub),
            },
            {
              find: /^@aozora\/playground-ui$/,
              replacement: fileURLToPath(
                new URL('../playground-ui/src/index.ts', root),
              ),
            },
            {
              find: /^@aozora\/playground-ui\/storage$/,
              replacement: fileURLToPath(
                new URL('../playground-ui/src/storage.ts', root),
              ),
            },
            {
              find: /^@aozora\/playground-ui\/testing$/,
              replacement: fileURLToPath(
                new URL(
                  '../playground-ui/src/testing/adapterContract.ts',
                  root,
                ),
              ),
            },
          ]
        : []),
      {
        find: /^lz-string$/,
        replacement: fileURLToPath(
          new URL('./node_modules/lz-string/libs/lz-string.js', root),
        ),
      },
      {
        find: /^aozora-wasm$/,
        replacement: fileURLToPath(mode === 'test' ? wasmStub : wasm),
      },
    ],
  },
  server: {
    host: '0.0.0.0',
    port: 5173,
    strictPort: true,
    fs: { allow: ['..'] },
  },
  preview: {
    host: '0.0.0.0',
    port: 5173,
    strictPort: true,
  },
  build: {
    target: ['es2022', 'safari16.2'],
    cssTarget: 'safari16.2',
    sourcemap: false,
    assetsInlineLimit: 0,
    cssCodeSplit: false,
    cssMinify: 'lightningcss',
    rollupOptions: {
      input: {
        main: fileURLToPath(new URL('./index.html', import.meta.url)),
        gallery: fileURLToPath(new URL('./gallery.html', import.meta.url)),
      },
      output: {
        manualChunks(id) {
          if (
            /macro-(.*)\.css$/.test(id) ||
            /@react-spectrum\/s2\/.*\.css$/.test(id)
          ) {
            return 's2-styles';
          }
          if (
            // Rolldown can hoist ActionMenu's Menu subtree into the main
            // artifact; isolate its narrow root to preserve ADR-0055's gate.
            id.endsWith(
              '/node_modules/react-aria-components/dist/private/Menu.mjs',
            )
          ) {
            return 'vendor-spectrum-menu';
          }
          if (
            id.includes('node_modules/@codemirror/') ||
            id.includes('node_modules/@lezer/') ||
            id.includes('node_modules/codemirror/')
          ) {
            return 'vendor-codemirror';
          }
          if (
            id.includes('node_modules/react/') ||
            id.includes('node_modules/react-dom/')
          ) {
            return 'vendor-react';
          }
          if (id.includes('node_modules/lz-string/')) {
            return 'vendor-lz-string';
          }
          return undefined;
        },
      },
    },
  },
  test: {
    include: [
      'src/**/*.test.{ts,tsx}',
      '../playground-ui/src/**/*.test.{ts,tsx}',
    ],
    exclude: [],
    environment: 'happy-dom',
    setupFiles: ['src/test-setup.ts'],
    server: {
      deps: {
        inline: [/@react-spectrum\/s2/],
      },
    },
    coverage: {
      provider: 'v8',
      reporter: ['text', 'json-summary'],
      allowExternal: true,
      exclude: [],
      include: [
        'src/adapter-engine.ts',
        'src/adapter.ts',
        'src/editor-controller.ts',
        'src/gallery-fixtures.ts',
        'src/editor/fuzzy.ts',
        'src/editor/parserState.ts',
        'src/editor/utils.ts',
        '**/playground-ui/src/catalog.ts',
        '**/playground-ui/src/PlaygroundApp.tsx',
        '**/playground-ui/src/share.ts',
        '**/playground-ui/src/storage.ts',
      ],
      thresholds: {
        statements: 85,
        branches: 75,
        functions: 90,
        lines: 85,
      },
    },
  },
}));
