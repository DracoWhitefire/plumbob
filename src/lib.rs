//! FRL link training state machine for HDMI 2.1.
//!
//! `plumbob` implements the Fixed Rate Link (FRL) training procedure defined in the
//! HDMI 2.1 specification. It defines the [`ScdcClient`] interface its dependencies
//! must satisfy and exposes `FrlTrainer` as the central entry point.
//!
//! # Features
//!
//! - **`alloc`** — enables `TrainingTrace` and `FrlTrainer::train_at_rate_traced`.
//! - **`std`** — implies `alloc`; no additional API surface.

#![no_std]
#![forbid(unsafe_code)]
#![deny(missing_docs)]

#[cfg(feature = "alloc")]
extern crate alloc;

mod scdc;
mod training;
mod types;

#[cfg(feature = "alloc")]
mod trace;

pub use types::{CedCount, CedCounters, FfeLevels, FrlConfig, LtpReq, TrainingStatus};

pub use scdc::ScdcClient;

pub use training::{TrainingConfig, TrainingError, TrainingOutcome};

// Further re-exports are added as each module is populated (steps 6–8).
