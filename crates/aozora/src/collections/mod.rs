//! Cache-friendly sorted-collection lookup.
//!
//! [`EytzingerMap`] (backed by an internal `EytzingerArray`) is the
//! substrate the AST registry layer ([`crate::syntax::ast`]) uses for
//! O(log n) "what node lives at this byte position?" queries during
//! HTML rendering and serialization.
//!
//! # Why Eytzinger?
//!
//! Standard `Vec::binary_search` walks a sorted array using midpoint
//! probes that are not predictable to the CPU prefetcher, so each
//! probe at depth d costs a cache miss once `d × sizeof(T)` exceeds
//! L1. The **Eytzinger layout** (Khuong & Morin, "Array Layouts for
//! Comparison-Based Searching", 2017) reorders the same data into BFS
//! traversal of the implicit binary search tree. Each probe at depth
//! d visits index `2k+1` or `2k+2` from index k, an access pattern
//! the prefetcher recognises and pipelines. The result: 2–3× faster
//! lookups at sizes ≥ L1 (~16k `u32`s) with no algorithmic change to
//! the calling code.
//!
//! ## Layout intuition
//!
//! For sorted input `[10, 20, 30, 40, 50, 60, 70]` (n=7), the
//! Eytzinger array is `[40, 20, 60, 10, 30, 50, 70]`:
//!
//! ```text
//!           40           ← index 0 (root)
//!          /  \
//!        20    60        ← indices 1, 2
//!        /\    /\
//!      10 30 50 70       ← indices 3, 4, 5, 6
//! ```
//!
//! Search algorithm:
//!
//! ```text
//! k = 0
//! while k < n:
//!     if target < data[k]: k = 2k + 1     (descend left)
//!     elif target > data[k]: k = 2k + 2   (descend right)
//!     else: return Some(k)
//! return None
//! ```
//!
//! The layout is isolated behind [`EytzingerMap`] so an alternative
//! (van Emde Boas, B+ tree) could be swapped in without touching the
//! syntax types that consume it.

#![forbid(unsafe_code)]

mod eytzinger;
mod map;

pub(crate) use map::EytzingerMap;
