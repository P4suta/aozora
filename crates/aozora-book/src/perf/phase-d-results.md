# Phase D — Sentinel enum + single-table registry results

Phase D collapsed four per-kind sentinel position tables into a
single position-keyed [`EytzingerMap`] dispatched through a
[`NodeRef`] enum. Pre-Phase-D the registry held independent
`inline` / `block_leaf` / `block_open` / `block_close` `EytzingerMap`s
and `Registry::node_at(pos)` swept them in declaration order with
four `if let Some(...) = table.get(&pos)` chains; the post-Phase-D
shape is one binary search per lookup, with the variant tag carried
on the entry itself.

## Structural changes

```
pre  : Registry { inline, block_leaf, block_open, block_close }   // 4× EytzingerMap
       node_at(pos) → 4-way if-let chain, ~4 binary searches worst-case

post : Registry { table: EytzingerMap<u32, NodeRef<'src>> }       // 1× EytzingerMap
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
corpus profiling pre-Phase-D showed registry lookups at ~12 % of
render time on bouten-heavy documents; the unified dispatch should
absorb roughly that fraction.

## Measurement procedure

Run before each minor release:

```bash
# Take a baseline against the previous release tag
git checkout v0.3.0
just samply-corpus --repeat 5 --out before.json.gz
git checkout -

# Take a current measurement
just samply-corpus --repeat 5 --out after.json.gz

# Diff at the function level
xtask trace compare before.json.gz after.json.gz
```

Numbers go in the table below at release time:

| Metric | Pre-Phase-D | Post-Phase-D | Δ |
|---|---|---|---|
| Render hot path (corpus median, ns/doc) | _to fill_ | _to fill_ | _to fill_ |
| Registry lookup CPU share (%) | _to fill_ | _to fill_ | _to fill_ |
| End-to-end parse + render p50 (ms/doc) | _to fill_ | _to fill_ | _to fill_ |

Repro environment recorded in [`perf/samply.md`]. Pin the host
CPU + corpus version + Rust toolchain so the table is comparable
across releases.

[`EytzingerMap`]: https://docs.rs/aozora-veb/latest/aozora_veb/struct.EytzingerMap.html
[`NodeRef`]: https://docs.rs/aozora-syntax/latest/aozora_syntax/borrowed/enum.NodeRef.html
[`perf/samply.md`]: ./samply.md
