# 0055. Spectrum playground UI is canonical in this repository

- Status: accepted
- Date: 2026-07-30
- Deciders: @P4suta
- Tags: playground, web, spectrum, accessibility, vendoring

## Context

The Aozora parser and Aozora Flavored Markdown renderer have separate WASM
engines, examples, guides, and document styles. Their authoring shells had
diverged in layout, persistence, sharing, accessibility, and UI technology.
Publishing a two-consumer shell to npm would create an unrelated release
stream, while consuming a floating Git branch would make deployments
irreproducible.

## Decision

This repository owns the private `playground-ui/` React 19 package. Adobe
Spectrum 2 is its only UI design system. The package uses Spectrum components,
icons, Provider, and the official style macro without `UNSAFE_*` overrides,
product colours, emoji controls, or selectors coupled to Spectrum internals.
Product CSS is limited to CodeMirror, renderer-owned notation, gallery
comparison layout, and pane sizing.

The shared package owns responsive application layout, localization,
persistence and migrations, explicit share URLs, diagnostics, outline,
commands, dialogs, and stale-analysis protection. A product implements
`PlaygroundAdapter` and retains the renderer HTML boundary.

Aozora Flavored Markdown vendors an allowlisted snapshot pinned to an Aozora
commit and package tree. A shared UI change is merged and verified here first,
then synchronized to the consumer. The consumer CI checks every allowed file
byte for byte against its lock.

The public playground is an authoring surface. Raw nodes, serialized output,
HTML source, and parser timing belong to API, CLI, and engine tests rather than
the public interface.

The notation gallery remains a separate `gallery.html` entry. It uses React
Spectrum for its shell and the real renderer plus the canonical notation
stylesheet for every displayed sample. Its `data-family` hooks and horizontal
and vertical computed-style browser tests remain compatibility contracts.

The production meta CSP retains every enforceable directive from the prior
playground. `frame-ancestors` is omitted because browsers ignore it in a meta
policy and GitHub Pages cannot add the response header required to enforce it;
the browser suite locks this distinction so it is not mistaken for protection.

## Verification

The package combines fake-adapter interaction tests with focused persistence
and sharing tests. Each product applies the same adapter contract.
Production-browser tests cover real WASM, desktop and mobile layouts, keyboard
focus, legacy state, CSP, self-hosted resources, WCAG 2.2 AA, gallery
rendering, and visual regression. Lighthouse CI and compressed bundle budgets
guard both HTML entries.

## Consequences

UI changes require two ordered pull requests, but the deployed sites share one
interaction model without a third release channel. Renderer and editor
differences remain local to each product adapter.
