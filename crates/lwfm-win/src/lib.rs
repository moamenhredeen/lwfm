//! The Win32 layer of lwfm.
//!
//! This is the only crate that depends on `windows` or contains `unsafe`.
//! Its public API uses its own types only, so the application never names a
//! Win32 type.

#![deny(clippy::undocumented_unsafe_blocks)]
#![deny(clippy::multiple_unsafe_ops_per_block)]

mod error;
mod gdi;
mod geom;
mod handle;
mod msg;
mod num;
mod wide;

pub mod controls;
pub mod fs;
pub mod input;
pub mod process;
pub mod sys;
pub mod text;
pub mod window;

pub use error::{Error, ErrorKind};
pub use geom::{Color, Dpi, Rect, Size};
