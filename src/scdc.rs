use crate::types::{CedCounters, FrlConfig, LtpRequests, SourceTestConfig, UpdateFlags};

/// Typed SCDC interface required by the link training state machine, one method per
/// register operation it performs.
///
/// Defined here so the state machine has no dependency on any specific SCDC
/// implementation. SCDC crates implement this trait; plumbob calls it.
///
/// # Implementer responsibilities
///
/// plumbob treats every `Ok` return as correct and does not re-validate it.
/// Implementers are responsible for:
///
/// - **Correct register mapping.** Returned values must reflect the sink's registers at
///   the time of the read. An undefined request value (0x9–0xD) in `Status_Flags_1/2`
///   has no [`LtpReq`](crate::LtpReq) variant and must be returned as an error.
///
/// - **Bus-level error handling.** Any I²C/DDC timeout, NACK or protocol error must
///   surface as `Err(Self::Error)`. Returning a zeroed or default `Ok` value in place of
///   an error corrupts the state machine: a zeroed [`LtpRequests`] reads as every lane
///   passing training.
///
/// - **Polling cadence.** plumbob polls [`read_flt_ready`](Self::read_flt_ready) in
///   LTS:2 and [`read_update_flags`](Self::read_update_flags) in LTS:3 and LTS:P in a
///   tight loop bounded by the poll limits in `TrainingConfig`. The implementer controls
///   how long each call takes and therefore how much wall time a poll represents; the
///   default limits assume one poll every 2 ms. Sleeping or yielding inside these
///   methods is the correct place to enforce that interval.
pub trait ScdcClient {
    /// Error type returned by SCDC operations.
    type Error;

    /// Read `FLT_ready` (`Status_Flags_0` bit 6).
    fn read_flt_ready(&mut self) -> Result<bool, Self::Error>;

    /// Read the `Update_0` flags the state machine uses (bits 3–5).
    fn read_update_flags(&mut self) -> Result<UpdateFlags, Self::Error>;

    /// Clear the given `Update_0` flags (write 1 to clear). Flags that are `false` are
    /// left as they are.
    fn clear_update_flags(&mut self, flags: UpdateFlags) -> Result<(), Self::Error>;

    /// Read the per-lane link training requests from `Status_Flags_1/2`.
    fn read_ltp_requests(&mut self) -> Result<LtpRequests, Self::Error>;

    /// Read `Source_Test_Configuration` (0x35).
    fn read_source_test_config(&mut self) -> Result<SourceTestConfig, Self::Error>;

    /// Write `Config_0` with read requests disabled and `FLT_no_retrain` clear.
    fn write_config_0_defaults(&mut self) -> Result<(), Self::Error>;

    /// Write `Config_1`: the FRL rate and FFE levels.
    fn write_frl_config(&mut self, config: FrlConfig) -> Result<(), Self::Error>;

    /// Read the per-lane character error counters, for diagnostics.
    ///
    /// Each lane's [`CedCount`](crate::CedCount) must be `None` when its validity bit is
    /// not set.
    fn read_ced(&mut self) -> Result<CedCounters, Self::Error>;
}
