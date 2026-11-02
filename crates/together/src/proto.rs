//! Wire types generated from `proto/listentogether.proto` by `build.rs`.
//!
//! The upstream schema is copied verbatim from the Android app, so field
//! numbers and types are identical. Generated code is kept private; the crate
//! re-exports the message types at the root.

#![allow(clippy::all)]

include!(concat!(env!("OUT_DIR"), "/listentogether.rs"));
