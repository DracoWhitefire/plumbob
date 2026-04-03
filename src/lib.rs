//! FRL link training state machine for HDMI 2.1.
//!
//! `plumbob` implements the Fixed Rate Link (FRL) training procedure defined in the
//! HDMI 2.1 specification. It defines the [`ScdcClient`] interface its dependencies
//! must satisfy and exposes [`FrlTrainer`] as the central entry point.
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
mod trace;
mod training;
mod types;

pub use scdc::ScdcClient;
pub use training::{FrlTrainer, TrainingConfig, TrainingError, TrainingOutcome};
pub use types::{CedCount, CedCounters, FfeLevels, FrlConfig, LtpReq, TrainingStatus};

#[cfg(feature = "alloc")]
pub use trace::{TrainingEvent, TrainingTrace};
