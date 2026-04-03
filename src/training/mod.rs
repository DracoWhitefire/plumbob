use display_types::cea861::hdmi_forum::HdmiForumFrl;
use hdmi_hal::phy::HdmiPhy;

#[cfg(feature = "alloc")]
use alloc::vec::Vec;

use crate::scdc::ScdcClient;
use crate::trace::TrainingEvent;
use crate::types::{FfeLevels, FrlConfig, LtpReq, TrainingStatus};

/// Outcome of a training attempt at a single FRL rate.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrainingOutcome {
    /// All lanes satisfied. The link is ready at this rate.
    Success {
        /// The FRL rate at which training succeeded.
        achieved_rate: HdmiForumFrl,
    },
    /// Training did not converge within the configured timeout.
    ///
    /// The caller should retry at a lower rate or fall back to TMDS.
    FallbackRequired,
}

/// Hard error that terminated a training attempt.
///
/// Distinct from [`TrainingOutcome::FallbackRequired`]: this means something
/// failed at the I/O level, not that the link simply did not train at this rate.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrainingError<ScdcErr, PhyErr> {
    /// The `ScdcClient` returned an error.
    Scdc(ScdcErr),
    /// The PHY returned an error.
    Phy(PhyErr),
}

/// Per-attempt training configuration.
///
/// Construct via [`TrainingConfig::default`] and override individual fields as
/// needed for your polling cadence and hardware constraints.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TrainingConfig {
    /// FFE levels advertised to the sink in Config_0.
    pub ffe_levels: FfeLevels,
    /// Whether to set DSC_FRL_Max in Config_0.
    ///
    /// Set to `true` when the negotiated configuration requires DSC transport.
    /// plumbob passes this through into `FrlConfig` without interpreting it.
    /// Defaults to `false`.
    pub dsc_frl_max: bool,
    /// Maximum number of polls to attempt while waiting for `flt_ready` (phase 2).
    ///
    /// The loop reads the training status register up to this many times. If
    /// `flt_ready` has not been asserted after exactly `flt_ready_timeout` polls,
    /// the attempt returns [`TrainingOutcome::FallbackRequired`]. A value of `0`
    /// means no polls are attempted and the phase times out immediately.
    pub flt_ready_timeout: u32,
    /// Maximum number of polls to attempt while waiting for `frl_start` (phase 3).
    ///
    /// Identical semantics to [`flt_ready_timeout`](Self::flt_ready_timeout): at
    /// most `frl_start_timeout` polls are made before the attempt returns
    /// [`TrainingOutcome::FallbackRequired`].
    pub frl_start_timeout: u32,
    /// Maximum number of poll iterations in the LTP training loop (phase 4).
    ///
    /// Identical semantics to [`flt_ready_timeout`](Self::flt_ready_timeout): at
    /// most `ltp_timeout` iterations run before the attempt returns
    /// [`TrainingOutcome::FallbackRequired`].
    pub ltp_timeout: u32,
}

impl Default for TrainingConfig {
    fn default() -> Self {
        Self {
            ffe_levels: FfeLevels::Ffe0,
            dsc_frl_max: false,
            flt_ready_timeout: 1000,
            frl_start_timeout: 1000,
            ltp_timeout: 1000,
        }
    }
}

/// The central training type. Owns an `ScdcClient` and an `HdmiPhy`.
///
/// `FrlTrainer` is reusable across multiple `train_at_rate` calls. A caller
/// performing rate fallback calls `train_at_rate` repeatedly on the same trainer,
/// stepping down through FRL tiers, without reconstructing it between attempts.
/// Use `into_parts` to recover the SCDC client and PHY when training is finished.
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

    /// Runs the full four-phase FRL training sequence at the given rate.
    ///
    /// Returns [`TrainingOutcome::Success`] when all lanes satisfy their LTP
    /// requests, or [`TrainingOutcome::FallbackRequired`] if any phase times out.
    /// A [`TrainingError`] is returned only on hard I/O failures.
    pub fn train_at_rate(
        &mut self,
        rate: HdmiForumFrl,
        config: &TrainingConfig,
    ) -> Result<TrainingOutcome, TrainingError<C::Error, P::Error>> {
        self.train_inner(rate, config, &mut |_| {})
    }

    /// Like [`FrlTrainer::train_at_rate`], but also returns a `TrainingTrace`
    /// recording the full event sequence.
    #[cfg(feature = "alloc")]
    pub fn train_at_rate_traced(
        &mut self,
        rate: HdmiForumFrl,
        config: &TrainingConfig,
    ) -> Result<(TrainingOutcome, crate::trace::TrainingTrace), TrainingError<C::Error, P::Error>>
    {
        let mut events = Vec::new();
        let outcome = self.train_inner(rate, config, &mut |e| events.push(e))?;
        Ok((
            outcome,
            crate::trace::TrainingTrace {
                rate,
                config: *config,
                events,
            },
        ))
    }

    /// Polls `read_training_status` until `condition` is satisfied or `timeout`
    /// iterations have elapsed. Emits the appropriate event via `record` in both
    /// cases. Returns `None` when the condition was met (caller should proceed) or
    /// `Some(TrainingOutcome::FallbackRequired)` on timeout.
    ///
    /// `i` counts polls attempted so far. At the point the condition is met it
    /// reflects how many prior reads failed; at timeout it equals `timeout` exactly.
    fn poll_until<F>(
        &mut self,
        timeout: u32,
        condition: impl Fn(&TrainingStatus) -> bool,
        on_success: impl FnOnce(u32) -> TrainingEvent,
        on_timeout: impl FnOnce(u32) -> TrainingEvent,
        record: &mut F,
    ) -> Result<Option<TrainingOutcome>, TrainingError<C::Error, P::Error>>
    where
        F: FnMut(TrainingEvent),
    {
        let mut i = 0u32;
        loop {
            let status = self
                .scdc
                .read_training_status()
                .map_err(TrainingError::Scdc)?;
            if condition(&status) {
                record(on_success(i));
                return Ok(None);
            }
            i += 1;
            if i >= timeout {
                record(on_timeout(i));
                return Ok(Some(TrainingOutcome::FallbackRequired));
            }
        }
    }

    /// Core four-phase training sequence.
    ///
    /// `record` is called with each [`TrainingEvent`] as it occurs. Pass
    /// `&mut |_| {}` for the non-traced path; the compiler eliminates the
    /// call entirely in optimised builds.
    fn train_inner<F>(
        &mut self,
        rate: HdmiForumFrl,
        config: &TrainingConfig,
        record: &mut F,
    ) -> Result<TrainingOutcome, TrainingError<C::Error, P::Error>>
    where
        F: FnMut(TrainingEvent),
    {
        // Phase 1 — Configuration
        self.scdc
            .write_frl_config(FrlConfig {
                rate,
                ffe_levels: config.ffe_levels,
                dsc_frl_max: config.dsc_frl_max,
            })
            .map_err(TrainingError::Scdc)?;
        self.phy.set_frl_rate(rate).map_err(TrainingError::Phy)?;
        record(TrainingEvent::RateConfigured {
            rate,
            ffe_levels: config.ffe_levels,
        });

        // Phase 2 — Readiness: poll until the sink asserts flt_ready.
        if let Some(outcome) = self.poll_until(
            config.flt_ready_timeout,
            |s| s.flt_ready,
            |i| TrainingEvent::FltReadyReceived {
                after_iterations: i,
            },
            |i| TrainingEvent::FltReadyTimeout {
                iterations_elapsed: i,
            },
            record,
        )? {
            return Ok(outcome);
        }

        // Phase 3 — Initiation: poll until the sink asserts frl_start.
        if let Some(outcome) = self.poll_until(
            config.frl_start_timeout,
            |s| s.frl_start,
            |i| TrainingEvent::FrlStartReceived {
                after_iterations: i,
            },
            |i| TrainingEvent::FrlStartTimeout {
                iterations_elapsed: i,
            },
            record,
        )? {
            return Ok(outcome);
        }

        // Phase 4 — LTP loop.
        self.ltp_loop(rate, config, record)
    }

    /// Phase 4: drive LTP patterns until the sink signals all lanes satisfied.
    ///
    /// `read_ced` is called on each iteration; it will feed per-lane equalization
    /// adjustments once `LaneEqParams` fields are defined in hdmi-hal.
    /// `LtpPatternRequested` is emitted only on transitions, not every poll.
    fn ltp_loop<F>(
        &mut self,
        rate: HdmiForumFrl,
        config: &TrainingConfig,
        record: &mut F,
    ) -> Result<TrainingOutcome, TrainingError<C::Error, P::Error>>
    where
        F: FnMut(TrainingEvent),
    {
        let mut i = 0u32;
        let mut last_ltp: Option<LtpReq> = None;
        loop {
            let status = self
                .scdc
                .read_training_status()
                .map_err(TrainingError::Scdc)?;
            if status.ltp_req == LtpReq::None {
                record(TrainingEvent::AllLanesSatisfied {
                    after_iterations: i,
                });
                return Ok(TrainingOutcome::Success {
                    achieved_rate: rate,
                });
            }
            if Some(status.ltp_req) != last_ltp {
                record(TrainingEvent::LtpPatternRequested {
                    pattern: status.ltp_req,
                });
                last_ltp = Some(status.ltp_req);
            }
            let _ced = self.scdc.read_ced().map_err(TrainingError::Scdc)?;
            self.phy
                .send_ltp(status.ltp_req.into())
                .map_err(TrainingError::Phy)?;
            i += 1;
            if i >= config.ltp_timeout {
                record(TrainingEvent::LtpLoopTimeout {
                    iterations_elapsed: i,
                });
                return Ok(TrainingOutcome::FallbackRequired);
            }
        }
    }
}

#[cfg(test)]
mod sim;

#[cfg(test)]
mod tests;
