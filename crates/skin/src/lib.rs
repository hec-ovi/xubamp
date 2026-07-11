//! Classic Winamp skin loading and decoding.
//!
//! Everything here is pure: bytes in, pixels or parsed structs out, with no I/O and
//! no global state, so each decoder is unit-tested in isolation and costs nothing
//! until called. Allocation is held to the minimum a result needs.

pub mod bmp;
