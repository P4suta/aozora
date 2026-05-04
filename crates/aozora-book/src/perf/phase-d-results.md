# Phase D — Sentinel enum + single-table registry

The single-table registry collapsed four per-kind sentinel position
tables into one position-keyed [`EytzingerMap`] dispatched through a
[`NodeRef`] enum. Before the refactor the registry held independent
`inline` / `block_leaf` / `block_open` / `block_close`
`EytzingerMap`s and `Registry::node_at(pos)` swept them in
declaration order with four `if let Some(...) = table.get(&pos)`
chains; the current shape is one binary search per lookup, with the
variant tag carried on the entry itself.

## Structural changes

```text
old  : Registry { inline, block_leaf, block_open, block_close }   // 4× EytzingerMap
       node_at(pos) → 4-way if-let chain, ~4 binary searches worst-case

now  : Registry { table: EytzingerMap<u32, NodeRef<'src>> }       // 1× EytzingerMap
       node_at(pos) → one binary search, NodeRef variant tags the kind
```

Renderers (`crates/aozora-render/src/html.rs`,
`crates/aozora-render/src/serialize.rs`) replaced the parallel
4-way `if let Some(...) = registry.<kind>.get(...)` chains with
a single `(Structural, NodeRef)` cross-product `match` — the
compiler now enforces variant coverage at the call site.

## Expected runtime impact

Theoretical: per-lookup binary search count drops from ≤ 4 to 1.
Render hot path is dominated by registry lookups inside the
`memchr2_iter` loop in `html::render_into` (one lookup per PUA
sentinel hit), so the savings scale with sentinel density. Aozora
corpus profiling against the four-table layout showed registry
lookups at ~12 % of render time on bouten-heavy documents; the
unified dispatch should absorb roughly that fraction.

## Measuring before / after

The repro recipe lives in [perf/samply.md](./samply.md#workflow-recipes).
Numerical comparisons against the previous release are produced as
release-PR artefacts (the corpus-sweep run output in
`/tmp/aozora-corpus-<timestamp>.json.gz`, plus the diff produced by
`xtask trace compare`) and summarised in the CHANGELOG entry for
the release that lands the change. Pinned numbers in this page
would rot; the recipe + per-release artefact pair stays current
without an editing step here.

[`EytzingerMap`]: https://docs.rs/aozora-veb/latest/aozora_veb/struct.EytzingerMap.html
[`NodeRef`]: https://docs.rs/aozora-syntax/latest/aozora_syntax/borrowed/enum.NodeRef.html
