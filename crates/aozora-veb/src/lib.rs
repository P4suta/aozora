//! Cache-friendly sorted-collection lookup.
//!
//! This crate provides [`EytzingerArray`] and [`EytzingerMap`], the
//! substrate that the AST registry layer (`aozora-syntax`) uses for
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
//! # Why a separate crate?
//!
//! `aozora-syntax` is the consumer; pulling the algorithm into its own
//! crate lets us benchmark it standalone, swap implementations (vEB,
//! B+ tree) without touching syntax types, and keep `aozora-syntax`'s
//! dependency surface minimal.
//!
//! Naming note: per the 0.2.0 plan this crate started life targeted at
//! a van Emde Boas layout. Eytzinger ships first because its
//! implementation is straightforward, its constants are small, and its
//! cache-friendliness for our access pattern matches vEB within a small
//! constant. If a benchmark ever shows vEB winning, the public API
//! shape is stable enough that the swap is internal.

#![forbid(unsafe_code)]
#![no_std]

extern crate alloc;

mod eytzinger;
mod map;

pub use eytzinger::EytzingerArray;
pub use map::EytzingerMap;
