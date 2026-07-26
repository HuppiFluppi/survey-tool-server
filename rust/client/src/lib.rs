//! Client library for the survey tool API.
//!
//! Exposes one client per transport, gated behind the `grpc` and `rest` cargo
//! features. Each transport speaks the same API so callers can be ported
//! between them with minimal changes. The `grpc` feature is enabled by default;
//! `rest` is a placeholder for the not-yet-implemented REST transport.

#[cfg(feature = "grpc")]
pub mod grpc;

#[cfg(feature = "rest")]
pub mod rest;
