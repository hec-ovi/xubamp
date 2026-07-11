//! Classic Winamp skin loading and decoding.
//!
//! Everything here is pure: bytes in, pixels or parsed structs out, with no I/O and
//! no global state, so each decoder is unit-tested in isolation and costs nothing
//! until called. Allocation is held to the minimum a result needs.

pub mod bmp;
pub mod color;
pub mod container;
pub mod default_skin;
pub mod font;
pub mod model;
pub mod pledit;
pub mod region;
pub mod sprites;
pub mod textfont;
pub mod viscolor;

pub use default_skin::default_skin;
pub use model::Skin;

#[cfg(test)]
mod testkit;
