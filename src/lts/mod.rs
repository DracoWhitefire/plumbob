//! The link training state machine (LTS:2 → LTS:3 → LTS:P, with LTS:4 and LTS:L).
//!
//! Built alongside the original training module and not yet exported; the public API
//! switches over to it once it is complete.

// Nothing here is reachable from the public API until the switch-over.
#![allow(dead_code)]

pub(crate) mod scdc;
pub(crate) mod trace;
pub(crate) mod trainer;
pub(crate) mod types;

#[cfg(test)]
mod sim;
