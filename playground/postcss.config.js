// PostCSS pipeline for the playground bundle. Vite auto-discovers this file
// (vite.config.ts sets no `css.postcss`), so every bundled stylesheet — the
// playground's own CSS *and* the imported canonical `aozora-notation.css` —
// passes through autoprefixer at build time. That is what lets us keep
// stylelint's `property-no-vendor-prefix` ENABLED for playground source: we
// write no `-webkit-*` by hand and autoprefixer adds exactly the prefixes the
// `browserslist` in package.json calls for (today that is only
// `-webkit-text-size-adjust`, which every Safari still requires). The raw
// canonical sheet shipped to the VS Code extension / afm is a separate copy
// that never goes through here, so its hand-written prefixes are untouched.
export default { plugins: { autoprefixer: {} } };
