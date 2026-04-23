//! Block-annotation dispatch hook.
//!
//! Called from a single line added to `upstream/comrak/src/parser/block.rs`.
//! Responsible for recognising `［＃...］` starters and managing the paired-annotation
//! stack (`字下げ`, `地付き`, `割り注`, `罫囲み` etc.).

// M0 Spike: module exists so the hook is ready for wiring once upstream vendor lands.
