//! Algorithm-axis modules for trigger-byte scanning.
//!
//! Each module here implements a complete scanning algorithm
//! independently of the underlying SIMD instruction set. Per-platform
//! impls of the inner kernels live under [`crate::arch`] (added in
//! follow-up steps); each `arch::*Kernel` plugs into the same
//! `*Inner` trait that the algorithm here drives.
//!
//! The split is the spine of the redesign: "what algorithm" lives
//! here (one file per algorithm), "what instructions" lives under
//! `arch/`. A new platform port is one file under `arch/`; a new
//! algorithm is one file here. The two axes never tangle.

pub(crate) mod teddy;
