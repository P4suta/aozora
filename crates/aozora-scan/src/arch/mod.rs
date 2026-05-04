//! Platform-axis modules — per-target SIMD inner kernels for the
//! Teddy outer driver.
//!
//! Each architecture-specific submodule implements
//! [`crate::kernel::teddy::TeddyInner`] with the appropriate
//! intrinsics. The outer driver in [`crate::kernel::teddy`] stays
//! platform-agnostic and runs the same algorithm against any inner
//! kernel; new ports plug in by adding a file here.
//!
//! Cfg gating selects exactly the platform that currently builds —
//! every kernel module is compiled only when the underlying target
//! arch matches, so unused intrinsics never show up in the link.

#[cfg(target_arch = "x86_64")]
pub(crate) mod x86_64;
