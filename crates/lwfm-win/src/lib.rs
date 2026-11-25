//! The Win32 layer of lwfm.
//!
//! This is the only crate that depends on `windows` or contains `unsafe`.
//! Its public API uses its own types only, so the application never names a
//! Win32 type.

#![deny(clippy::undocumented_unsafe_blocks)]
#![deny(clippy::multiple_unsafe_ops_per_block)]
