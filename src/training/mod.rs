use core::future::Future;
use core::pin::pin;
use core::task::{Context, Poll, Waker};

use display_types::cea861::hdmi_forum::HdmiForumFrl;
use hdmi_hal::phy::{EqParams, FrlOutput, HdmiPhy, LanePatterns};

use crate::lts::{self, TrainingIo};
use crate::scdc::ScdcClient;
use crate::trace::TrainingEvent;
use crate::types::{FfeLevels, FrlConfig, LtpRequests, SourceTestConfig, UpdateFlags};
use crate::warning::Trained;

#[cfg(feature = "alloc")]
use crate::trace::TrainingTrace;
#[cfg(feature = "alloc")]
use alloc::vec::Vec;

/// Why a training attempt ended without a link.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FallbackReason {
    /// LTS:2: `FLT_ready` did not assert within the poll limit.
    FltReadyTimeout,
    /// LTS:3: the lanes did not pass within the poll limit.
    TrainingTimeout,
    /// LTS:P: `FRL_start` did not assert within the poll limit.
    FrlStartTimeout,
    /// LTS:4: the sink requested a lower rate than the last one in the list (or the
    /// list was empty).
    RatesExhausted,
    /// LTS:P: the sink requested retraining (`FLT_update`) after
    /// [`TrainingConfig::max_retrains`] retrains had been used. Also under
    /// `FLT_no_timeout`, where the poll limits would hold the link instead.
    RetrainsExhausted,
}

/// The result of a training attempt.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrainingOutcome {
    /// Training passed and the sink set `FRL_start`. The link is ready at this rate.
    Success {
        /// The FRL rate at which training succeeded.
        achieved_rate: HdmiForumFrl,
    },
    /// Training did not succeed at any of the rates.
    FallbackRequired {
        /// Why the attempt ended.
        reason: FallbackReason,
    },
    /// The sink set `FLT_no_timeout` — it is under compliance test — and plumbob's
    /// [`no_timeout_poll_cap`](TrainingConfig::no_timeout_poll_cap) ran out. The test
    /// equipment is in control of the link, so plumbob leaves it as it is, with no LTS:L:
    /// in LTS:3 or LTS:P the PHY and `Config_1` stay at `rate`; in LTS:2 nothing has been
    /// configured yet. The caller decides what comes next — keep the link up for the test,
    /// train again, or [`exit_to_tmds`](FrlTrainer::exit_to_tmds).
    NoTimeoutHold {
        /// The rate being trained when the cap ran out.
        rate: HdmiForumFrl,
    },
}

/// Hard error that terminated a training attempt.
///
/// Distinct from [`TrainingOutcome::FallbackRequired`]: this means something
/// failed at the I/O level, not that the link simply did not train at this rate.
///
/// After an SCDC or PHY error, plumbob performs LTS:L, returning both ends to TMDS as
/// after a fallback, unless [`TrainingConfig::exit_to_tmds_on_error`] is off. `exit` says
/// what happened. When LTS:L itself fails after a fallback, the error is
/// [`ExitFailed`](Self::ExitFailed).
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrainingError<ScdcErr, PhyErr> {
    /// The `ScdcClient` returned an error.
    Scdc {
        /// The error.
        error: ScdcErr,
        /// LTS:L after the error.
        exit: TmdsExit<ScdcErr, PhyErr>,
    },
    /// The PHY returned an error.
    Phy {
        /// The error.
        error: PhyErr,
        /// LTS:L after the error.
        exit: TmdsExit<ScdcErr, PhyErr>,
    },
    /// The attempt fell back for `reason`, and LTS:L then failed.
    ExitFailed {
        /// Why the attempt fell back.
        reason: FallbackReason,
        /// LTS:L's errors.
        error: ExitError<ScdcErr, PhyErr>,
    },
}

/// What LTS:L did after an SCDC or PHY error.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TmdsExit<ScdcErr, PhyErr> {
    /// LTS:L ran and every step succeeded: the sink and PHY are in TMDS.
    Exited,
    /// LTS:L was not run, because [`TrainingConfig::exit_to_tmds_on_error`] is off. Both
    /// ends are as the error left them.
    Skipped,
    /// LTS:L ran and at least one step failed.
    Failed(ExitError<ScdcErr, PhyErr>),
}

/// LTS:L failed: the first error of each end whose steps failed.
///
/// LTS:L attempts every step even when one fails, so an end with `None` completed its
/// steps and is in TMDS. At least one end has an error. Returned by
/// [`FrlTrainer::exit_to_tmds`] and [`lts::exit_to_tmds`], and carried by
/// [`TmdsExit::Failed`] and [`TrainingError::ExitFailed`].
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ExitError<ScdcErr, PhyErr> {
    /// The first error of LTS:L's SCDC steps (`Config_1`, `FLT_update`), if any.
    pub scdc: Option<ScdcErr>,
    /// The first error of LTS:L's PHY steps (patterns, rate), if any.
    pub phy: Option<PhyErr>,
}

/// Per-attempt training configuration.
///
/// Construct via [`TrainingConfig::default`] and override fields as needed. Poll limits
/// are exact counts: N means exactly N polls before the state gives up. The defaults
/// assume one poll every 2 ms.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TrainingConfig {
    /// Highest TxFFE level the source supports, written to `Config_1` (limited per rate).
    /// Default 3, as the AMD and Intel drivers advertise: the sink can then request up to
    /// three TxFFE raises per lane. Set it to 0 for a PHY that cannot apply TxFFE, so the
    /// source does not advertise levels it cannot deliver; requests to raise a level are
    /// then not acted on.
    pub ffe_levels: FfeLevels,
    /// Poll limit for `FLT_ready` in LTS:2. Default 50 (100 ms at 2 ms per poll).
    pub flt_ready_polls: u32,
    /// Poll limit for LTS:3. Default 100 (200 ms at 2 ms per poll).
    pub ltp_polls: u32,
    /// Poll limit for `FRL_start` in LTS:P. Default 100 (200 ms at 2 ms per poll, the
    /// `FRL_start` wait of the AMD and Intel drivers).
    pub frl_start_polls: u32,
    /// The LTS:2, LTS:3 and LTS:P poll limit while the sink sets `FLT_no_timeout`. When it
    /// runs out the attempt ends with [`TrainingOutcome::NoTimeoutHold`], leaving the link
    /// as it is. Default 500 (1 s at 2 ms per poll, the AMD driver's cap).
    pub no_timeout_poll_cap: u32,
    /// How many times one `train` call returns from LTS:P to LTS:3 when the sink requests
    /// retraining (`FLT_update`). The next request after that ends the attempt with
    /// [`FallbackReason::RetrainsExhausted`]; 0 falls back on the first one. Default 3,
    /// the AMD driver's retry count (which reruns the whole procedure rather than LTS:3).
    ///
    /// Unlike the poll limits, this bound holds under `FLT_no_timeout` as well: it is what
    /// guarantees that the LTS:P ↔ LTS:3 cycle ends, so exhausting it falls back rather
    /// than ending in [`TrainingOutcome::NoTimeoutHold`]. A compliance test that needs
    /// more retrains raises it.
    pub max_retrains: u32,
    /// Whether an SCDC or PHY error is followed by LTS:L, returning both ends to TMDS as a
    /// fallback does. Default `true`. Turn it off to leave the sink and PHY as the error
    /// left them, for example to inspect the sink's registers; [`TrainingError`] then
    /// reports [`TmdsExit::Skipped`].
    pub exit_to_tmds_on_error: bool,
}

impl Default for TrainingConfig {
    fn default() -> Self {
        Self {
            ffe_levels: FfeLevels::new(3).expect("3 is a valid FFE level"),
            flt_ready_polls: 50,
            ltp_polls: 100,
            frl_start_polls: 100,
            no_timeout_poll_cap: 500,
            max_retrains: 3,
            exit_to_tmds_on_error: true,
        }
    }
}

type Error<C, P> = TrainingError<<C as ScdcClient>::Error, <P as HdmiPhy>::Error>;

/// The central training type. Owns an `ScdcClient` and an `HdmiPhy`.
///
/// Reusable across training attempts; use [`into_parts`](Self::into_parts) to recover
/// the SCDC client and PHY when training is finished.
pub struct FrlTrainer<C, P> {
    scdc: C,
    phy: P,
}

impl<C: ScdcClient, P: HdmiPhy> FrlTrainer<C, P> {
    /// Constructs a new `FrlTrainer` owning the given SCDC client and PHY.
    pub fn new(scdc: C, phy: P) -> Self {
        Self { scdc, phy }
    }

    /// Consumes the trainer and returns the SCDC client and PHY.
    pub fn into_parts(self) -> (C, P) {
        (self.scdc, self.phy)
    }

    /// LTS:L on demand: stops the training patterns, returns the PHY to TMDS, turns FRL
    /// off in `Config_1` and clears `FLT_update` if it is set.
    ///
    /// For taking an FRL link down outside [`train`](Self::train) — when the display is
    /// disabled or unplugged, or before a mode change — and after an error with
    /// [`TrainingConfig::exit_to_tmds_on_error`] off. Safe to call in any state, and
    /// again. Every step is attempted even when one fails.
    pub fn exit_to_tmds(&mut self) -> Result<(), ExitError<C::Error, P::Error>> {
        let mut io = SyncIo {
            scdc: &mut self.scdc,
            phy: &mut self.phy,
        };
        block_on(lts::exit_to_tmds(&mut io, &mut |_| {}))
    }

    /// Trains at a single rate: `train(&[rate], config)`.
    pub fn train_at_rate(
        &mut self,
        rate: HdmiForumFrl,
        config: &TrainingConfig,
    ) -> Result<Trained, Error<C, P>> {
        self.train(&[rate], config)
    }

    /// Trains over `rates` in order, stepping down when the sink requests it.
    ///
    /// Returns [`TrainingOutcome::Success`] once the sink sets `FRL_start`, or
    /// [`TrainingOutcome::FallbackRequired`] with the reason the attempt ended. An empty
    /// list returns `FallbackRequired { reason: RatesExhausted }` without touching the
    /// sink. A [`TrainingError`] is returned only on SCDC or PHY failures.
    pub fn train(
        &mut self,
        rates: &[HdmiForumFrl],
        config: &TrainingConfig,
    ) -> Result<Trained, Error<C, P>> {
        self.train_with_events(rates, config, &mut |_| {})
    }

    /// Like [`train_at_rate`](Self::train_at_rate), and also returns a [`TrainingTrace`]
    /// of the attempt, whatever its result.
    #[cfg(feature = "alloc")]
    pub fn train_at_rate_traced(
        &mut self,
        rate: HdmiForumFrl,
        config: &TrainingConfig,
    ) -> (Result<Trained, Error<C, P>>, TrainingTrace) {
        self.train_traced(&[rate], config)
    }

    /// Like [`train`](Self::train), and also returns a [`TrainingTrace`] of the attempt.
    ///
    /// The trace is returned whatever the result, so an attempt that ended in a
    /// [`TrainingError`] can be explained as well: its events up to the error, then LTS:L's
    /// `ExitedToTmds` or `ExitToTmdsFailed`.
    #[cfg(feature = "alloc")]
    pub fn train_traced(
        &mut self,
        rates: &[HdmiForumFrl],
        config: &TrainingConfig,
    ) -> (Result<Trained, Error<C, P>>, TrainingTrace) {
        let mut events = Vec::new();
        let result = self.train_with_events(rates, config, &mut |event| events.push(event));
        (result, TrainingTrace::new(rates.to_vec(), *config, events))
    }

    /// Like [`train`](Self::train), calling `record` with each [`TrainingEvent`] as it
    /// occurs: the events [`train_traced`](Self::train_traced) collects, without an
    /// allocator. The caller decides what to keep — log each event, count them, or store
    /// the last few.
    pub fn train_with_events<F: FnMut(TrainingEvent)>(
        &mut self,
        rates: &[HdmiForumFrl],
        config: &TrainingConfig,
        record: &mut F,
    ) -> Result<Trained, Error<C, P>> {
        let mut io = SyncIo {
            scdc: &mut self.scdc,
            phy: &mut self.phy,
        };
        block_on(lts::run(&mut io, rates, config, record))
    }
}

/// [`TrainingIo`] over a sync `ScdcClient` and `HdmiPhy`. Each operation is the sync call
/// itself, so its future is ready the first time it is polled.
struct SyncIo<'a, C, P> {
    scdc: &'a mut C,
    phy: &'a mut P,
}

impl<C: ScdcClient, P: HdmiPhy> TrainingIo for SyncIo<'_, C, P> {
    type ScdcError = C::Error;
    type PhyError = P::Error;

    async fn read_flt_ready(&mut self) -> Result<bool, C::Error> {
        self.scdc.read_flt_ready()
    }

    async fn read_update_flags(&mut self) -> Result<UpdateFlags, C::Error> {
        self.scdc.read_update_flags()
    }

    async fn clear_update_flags(&mut self, flags: UpdateFlags) -> Result<(), C::Error> {
        self.scdc.clear_update_flags(flags)
    }

    async fn read_ltp_requests(&mut self) -> Result<LtpRequests, C::Error> {
        self.scdc.read_ltp_requests()
    }

    async fn read_source_test_config(&mut self) -> Result<SourceTestConfig, C::Error> {
        self.scdc.read_source_test_config()
    }

    async fn write_config_0_defaults(&mut self) -> Result<(), C::Error> {
        self.scdc.write_config_0_defaults()
    }

    async fn write_frl_config(&mut self, config: FrlConfig) -> Result<(), C::Error> {
        self.scdc.write_frl_config(config)
    }

    async fn set_frl_rate(&mut self, rate: HdmiForumFrl) -> Result<(), P::Error> {
        self.phy.set_frl_rate(rate)
    }

    async fn send_ltp(&mut self, patterns: LanePatterns) -> Result<(), P::Error> {
        self.phy.send_ltp(patterns)
    }

    async fn set_frl_output(&mut self, output: FrlOutput) -> Result<(), P::Error> {
        self.phy.set_frl_output(output)
    }

    async fn adjust_equalization(&mut self, params: EqParams) -> Result<(), P::Error> {
        self.phy.adjust_equalization(params)
    }
}

/// Runs a future that never waits to completion, without an executor.
///
/// Over [`SyncIo`] every operation completes when first polled, so [`lts::run`] is ready
/// after one poll.
fn block_on<T>(future: impl Future<Output = T>) -> T {
    let mut future = pin!(future);
    ready(
        future
            .as_mut()
            .poll(&mut Context::from_waker(Waker::noop())),
    )
}

/// The output of a future polled once; a `Pending` here would be a bug in plumbob.
fn ready<T>(poll: Poll<T>) -> T {
    match poll {
        Poll::Ready(output) => output,
        Poll::Pending => unreachable!("plumbob's sync training waited on I/O"),
    }
}

#[cfg(test)]
mod sim;

#[cfg(test)]
mod tests;
