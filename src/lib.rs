//! FRL link training state machine for HDMI 2.1.
//!
//! `plumbob` implements the Fixed Rate Link (FRL) training procedure defined in the
//! HDMI 2.1 specification. It defines the [`ScdcClient`] interface its dependencies
//! must satisfy and exposes [`FrlTrainer`] as the central entry point.
//!
//! The state machine itself is [`lts::run`], an `async fn` that performs no I/O of its
//! own. [`FrlTrainer`] drives it synchronously, without an async runtime; `plumbob-async`
//! drives the same function asynchronously.
//!
//! # Features
//!
//! - **`alloc`** — enables `TrainingTrace`, `FrlTrainer::train_traced` and
//!   `FrlTrainer::train_at_rate_traced`.
//! - **`std`** — implies `alloc`; no additional API surface.

#![no_std]
#![forbid(unsafe_code)]
#![deny(missing_docs)]

#[cfg(feature = "alloc")]
extern crate alloc;

pub mod lts;
mod scdc;
mod trace;
mod training;
mod types;

pub use scdc::ScdcClient;
pub use training::{FallbackReason, FrlTrainer, TrainingConfig, TrainingError, TrainingOutcome};
pub use types::{
    CedCount, CedCounters, FfeLevels, FrlConfig, LtpReq, LtpRequests, SourceTestConfig, UpdateFlags,
};

pub use trace::TrainingEvent;
#[cfg(feature = "alloc")]
pub use trace::TrainingTrace;
