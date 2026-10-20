use crate::types::{ClearableUpdateFlags, FrlConfig, LtpRequests, SourceTestConfig, UpdateFlags};

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
///   is returned as [`LtpReq::Reserved`](crate::LtpReq::Reserved) with its raw value,
///   not as an error: whether it matters depends on the lanes in use, so plumbob decides
///   (it leaves the lane as it was and reports a `TrainingWarning`).
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
///
/// - **Stopping early.** The polled methods are also where a wall-clock deadline, a
///   cancel flag or any other reason to stop belongs. Return one error of your own from a
///   polled read, then let every later call through: training stops, LTS:L returns the
///   link to TMDS, and `train` returns
///   [`TrainingError::Scdc`](crate::TrainingError::Scdc) with that error and
///   `TmdsExit::Exited`. LTS:L's own `read_update_flags` is a single read, not a poll, but
///   it is the same method, so never fail a read made after a
///   [`write_frl_config`](Self::write_frl_config) with `HdmiForumFrl::NotSupported`
///   (LTS:L's first SCDC write; a later write with an FRL rate starts training again): a
///   deadline that passes while the attempt is already ending then lets it end with its
///   own result. The exit is `TmdsExit::Skipped` with
///   `TrainingConfig::exit_to_tmds_on_error` off, and `TmdsExit::HeldUnderTest` (the link
///   left as it is) while the sink sets `FLT_no_timeout`.
pub trait ScdcClient {
    /// Error type returned by SCDC operations.
    type Error;

    /// Read `FLT_ready` (`Status_Flags_0` bit 6).
    fn read_flt_ready(&mut self) -> Result<bool, Self::Error>;

    /// Read `Update_0` (0x10): all seven flags, whether or not the state machine acts on
    /// them.
    fn read_update_flags(&mut self) -> Result<UpdateFlags, Self::Error>;

    /// Clear the given `Update_0` flags (write 1 to clear). Flags that are `false` are
    /// left as they are; plumbob passes only the flags it acts on.
    fn clear_update_flags(&mut self, flags: ClearableUpdateFlags) -> Result<(), Self::Error>;

    /// Read the per-lane link training requests from `Status_Flags_1/2`. Undefined values
    /// (0x9–0xD) are [`LtpReq::Reserved`](crate::LtpReq::Reserved), not an error: whether
    /// they matter depends on the lanes in use, which the state machine knows.
    fn read_ltp_requests(&mut self) -> Result<LtpRequests, Self::Error>;

    /// Read `Source_Test_Configuration` (0x35).
    fn read_source_test_config(&mut self) -> Result<SourceTestConfig, Self::Error>;

    /// Write `Config_0` with read requests disabled and `FLT_no_retrain` clear.
    fn write_config_0_defaults(&mut self) -> Result<(), Self::Error>;

    /// Write `Config_1`: the FRL rate and FFE levels.
    fn write_frl_config(&mut self, config: FrlConfig) -> Result<(), Self::Error>;
}
