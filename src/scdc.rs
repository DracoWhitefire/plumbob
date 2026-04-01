use crate::types::{CedCounters, FrlConfig, TrainingStatus};

/// Typed SCDC interface required by the link training state machine.
///
/// Defined here so the state machine has no dependency on any specific SCDC
/// implementation. SCDC crates implement this trait; plumbob calls it.
///
/// The three methods map directly onto the register groups the training
/// procedure accesses: configuration write, status poll, and CED read.
pub trait ScdcClient {
    /// Error type returned by SCDC operations.
    type Error;

    /// Write FRL rate and configuration to Config_0.
    fn write_frl_config(&mut self, config: FrlConfig) -> Result<(), Self::Error>;

    /// Read `flt_ready`, `frl_start`, and `ltp_req` from Status_Flags.
    fn read_training_status(&mut self) -> Result<TrainingStatus, Self::Error>;

    /// Read per-lane character error counts for equalization feedback.
    fn read_ced(&mut self) -> Result<CedCounters, Self::Error>;
}
