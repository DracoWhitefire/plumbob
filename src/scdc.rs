use crate::types::{CedCounters, FrlConfig, TrainingStatus};

/// Typed SCDC interface required by the link training state machine.
///
/// Defined here so the state machine has no dependency on any specific SCDC
/// implementation. SCDC crates implement this trait; plumbob calls it.
///
/// The three methods map directly onto the register groups the training
/// procedure accesses: configuration write, status poll, and CED read.
///
/// # Implementer responsibilities
///
/// plumbob treats every `Ok` return as correct. It does **not** re-validate
/// the contents of [`TrainingStatus`] or [`CedCounters`] after receiving them.
/// Implementers are responsible for:
///
/// - **Correct register mapping.** Fields in the returned structs must
///   faithfully reflect the hardware register state at the time of the read.
///   In particular, [`TrainingStatus::ltp_req`] must match the sink's
///   `LTP_Req` field, and each [`CedCounters`] lane must be `None` when
///   the corresponding validity bit is not set in the hardware register.
///
/// - **Bus-level error handling.** Any I2C/DDC timeout, NACK, or protocol
///   error must surface as `Err(Self::Error)`. Returning a zeroed or default
///   `Ok` value in place of a real error will silently corrupt the training
///   state machine (e.g. a zeroed `TrainingStatus` looks like all-lanes-
///   satisfied and causes a false `TrainingOutcome::Success`).
///
/// - **Polling cadence.** plumbob calls these methods in a tight loop bounded
///   by the timeouts in [`TrainingConfig`](crate::TrainingConfig). The
///   implementer controls how long each call takes and therefore how much wall
///   time a single iteration represents. Sleeping or yielding inside these
///   methods is the correct place to enforce the inter-poll delay required by
///   the HDMI 2.1 specification.
pub trait ScdcClient {
    /// Error type returned by SCDC operations.
    type Error;

    /// Write FRL rate and configuration to Config_0.
    fn write_frl_config(&mut self, config: FrlConfig) -> Result<(), Self::Error>;

    /// Read `flt_ready`, `frl_start`, and `ltp_req` from Status_Flags.
    fn read_training_status(&mut self) -> Result<TrainingStatus, Self::Error>;

    /// Read per-lane character error counts for equalization feedback.
    ///
    /// Each lane's [`CedCount`](crate::CedCount) must be `None` when the
    /// validity bit for that lane is not asserted in the hardware register.
    fn read_ced(&mut self) -> Result<CedCounters, Self::Error>;
}
