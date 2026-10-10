//! The link training state machine, independent of how its I/O is performed.
//!
//! [`run`] is the whole procedure — LTS:2 → LTS:3 → LTS:P, with LTS:4 and LTS:L — written
//! once as an `async fn` over [`TrainingIo`]. It performs no I/O itself: every SCDC and PHY
//! operation goes through the `TrainingIo` it is given.
//!
//! [`FrlTrainer`](crate::FrlTrainer) drives it synchronously over an
//! [`ScdcClient`](crate::ScdcClient) and an `HdmiPhy`, without an async runtime: its I/O
//! completes immediately, so `run` never waits. `plumbob-async` drives the same `run` over
//! async I/O. Most users want `FrlTrainer`; this module is for writing such a driver.

use display_types::cea861::hdmi_forum::HdmiForumFrl;
use hdmi_hal::phy::{EqParams, FrlOutput, LaneEqParams, LanePatterns, LtpPattern, TxFfeLevel};

use crate::trace::TrainingEvent;
use crate::training::{
    ExitError, FallbackReason, TmdsExit, TrainingConfig, TrainingError, TrainingOutcome,
};
use crate::types::{FfeLevels, FrlConfig, LtpReq, LtpRequests, SourceTestConfig, UpdateFlags};
use crate::warning::Trained;

/// The SCDC and PHY operations the link training state machine performs.
///
/// The SCDC methods correspond to [`ScdcClient`](crate::ScdcClient)'s (without
/// `read_ced`, which training does not use) and the PHY methods to `HdmiPhy`'s; see those
/// traits for what each one does and what implementations must guarantee.
// `async fn` in a public trait does not promise `Send` futures. That is intended: the
// state machine targets single-threaded executors and the sync driver, as in
// `hdmi-hal-async`.
#[allow(async_fn_in_trait)]
pub trait TrainingIo {
    /// Error type of the SCDC operations.
    type ScdcError;
    /// Error type of the PHY operations.
    type PhyError;

    /// Read `FLT_ready`.
    async fn read_flt_ready(&mut self) -> Result<bool, Self::ScdcError>;
    /// Read the `Update_0` training flags.
    async fn read_update_flags(&mut self) -> Result<UpdateFlags, Self::ScdcError>;
    /// Clear the given `Update_0` flags.
    async fn clear_update_flags(&mut self, flags: UpdateFlags) -> Result<(), Self::ScdcError>;
    /// Read the per-lane link training requests.
    async fn read_ltp_requests(&mut self) -> Result<LtpRequests, Self::ScdcError>;
    /// Read `Source_Test_Configuration`.
    async fn read_source_test_config(&mut self) -> Result<SourceTestConfig, Self::ScdcError>;
    /// Write `Config_0` with read requests disabled and `FLT_no_retrain` clear.
    async fn write_config_0_defaults(&mut self) -> Result<(), Self::ScdcError>;
    /// Write `Config_1`.
    async fn write_frl_config(&mut self, config: FrlConfig) -> Result<(), Self::ScdcError>;
    /// Select the FRL rate (or TMDS) on the PHY.
    async fn set_frl_rate(&mut self, rate: HdmiForumFrl) -> Result<(), Self::PhyError>;
    /// Drive the given link training patterns, one per lane.
    async fn send_ltp(&mut self, patterns: LanePatterns) -> Result<(), Self::PhyError>;
    /// Select what the transmitter sends on the FRL lanes.
    async fn set_frl_output(&mut self, output: FrlOutput) -> Result<(), Self::PhyError>;
    /// Apply per-lane equalization (TxFFE levels).
    async fn adjust_equalization(&mut self, params: EqParams) -> Result<(), Self::PhyError>;
}

/// Runs one training attempt over `rates`, in order, through `io`.
///
/// This is the procedure behind [`FrlTrainer::train`](crate::FrlTrainer::train): the same
/// outcomes, errors and events, with `record` called for each [`TrainingEvent`] as it
/// occurs. An empty `rates` returns `FallbackRequired { reason: RatesExhausted }` without
/// any I/O.
///
/// The outcome comes with the [`TrainingWarning`](crate::TrainingWarning)s the attempt
/// produced, built from the same events `record` receives.
///
/// On an SCDC or PHY error, `run` performs LTS:L before returning, unless
/// [`TrainingConfig::exit_to_tmds_on_error`] is off; the returned [`TrainingError`] says
/// whether both ends reached TMDS.
pub async fn run<Io: TrainingIo, F: FnMut(TrainingEvent)>(
    io: &mut Io,
    rates: &[HdmiForumFrl],
    config: &TrainingConfig,
    record: &mut F,
) -> Result<Trained, Error<Io>> {
    // Collects the warnings while the attempt runs; the outcome is set at the end.
    let mut trained = Trained::new(TrainingOutcome::FallbackRequired {
        reason: FallbackReason::RatesExhausted,
    });
    let mut record = |event: TrainingEvent| {
        trained.observe(&event);
        record(event);
    };
    let outcome = Machine { io }.run(rates, config, &mut record).await?;
    trained.outcome = outcome;
    Ok(trained)
}

type Error<Io> = TrainingError<<Io as TrainingIo>::ScdcError, <Io as TrainingIo>::PhyError>;

/// An SCDC or PHY error, before LTS:L has run.
enum Fault<ScdcErr, PhyErr> {
    Scdc(ScdcErr),
    Phy(PhyErr),
}

type Failure<Io> = Fault<<Io as TrainingIo>::ScdcError, <Io as TrainingIo>::PhyError>;

/// Performs LTS:L through `io`: stops the training patterns, returns the PHY to TMDS,
/// turns FRL off in `Config_1` and clears `FLT_update` if it is set.
///
/// This is [`FrlTrainer::exit_to_tmds`](crate::FrlTrainer::exit_to_tmds), with `record`
/// called with `ExitedToTmds` or `ExitToTmdsFailed`. Every step is attempted even when
/// one fails; on failure, the error has each end's first error.
pub async fn exit_to_tmds<Io: TrainingIo, F: FnMut(TrainingEvent)>(
    io: &mut Io,
    record: &mut F,
) -> Result<(), ExitError<Io::ScdcError, Io::PhyError>> {
    Machine { io }.exit_to_tmds(record).await
}

pub(crate) const FLT_UPDATE: UpdateFlags = UpdateFlags {
    source_test_update: false,
    frl_start: false,
    flt_update: true,
};

pub(crate) const FRL_START: UpdateFlags = UpdateFlags {
    source_test_update: false,
    frl_start: true,
    flt_update: false,
};

pub(crate) const SOURCE_TEST_UPDATE: UpdateFlags = UpdateFlags {
    source_test_update: true,
    frl_start: false,
    flt_update: false,
};

/// The number of lanes in use at `rate`: 3 for the 3-lane rates, 4 otherwise.
fn lane_count(rate: HdmiForumFrl) -> usize {
    match rate {
        HdmiForumFrl::Rate3Gbps3Lanes | HdmiForumFrl::Rate6Gbps3Lanes => 3,
        _ => 4,
    }
}

/// The same pattern (or none) on every lane in use.
pub(crate) fn uniform(count: usize, pattern: Option<LtpPattern>) -> LanePatterns {
    let lane = |i| if i < count { pattern } else { None };
    LanePatterns {
        lane0: lane(0),
        lane1: lane(1),
        lane2: lane(2),
        lane3: lane(3),
    }
}

/// The equalization settings for one lane at the given TxFFE level.
fn lane_eq(level: u8) -> LaneEqParams {
    let mut params = LaneEqParams::default();
    // Levels are kept at or below `FfeLevels::MAX` (7), so the fallback is never used.
    params.tx_ffe_level = TxFfeLevel::new(level).unwrap_or(TxFfeLevel::MAX);
    params
}

/// Each lane's current pattern and TxFFE level, held by plumbob. The PHY applies exactly
/// what it is given, and the lanes always hold what it was last told: LTS:P's stop clears
/// the patterns, LTS:4 resets everything.
#[derive(Debug)]
pub(crate) struct Lanes {
    count: usize,
    patterns: [Option<LtpPattern>; 4],
    levels: [u8; 4],
}

impl Lanes {
    /// Every lane with no pattern and TxFFE level 0.
    pub(crate) fn new(rate: HdmiForumFrl) -> Self {
        Self {
            count: lane_count(rate),
            patterns: [None; 4],
            levels: [0; 4],
        }
    }

    /// Stops every lane's pattern, keeping its TxFFE level: what LTS:P tells the PHY.
    fn stop_patterns(&mut self) {
        self.patterns = [None; 4];
    }

    /// Updates each lane from the sink's request for it, recording each TxFFE raise.
    /// Returns whether any TxFFE level changed.
    fn apply<F: FnMut(TrainingEvent)>(
        &mut self,
        requests: LtpRequests,
        no_timeout: bool,
        max_level: u8,
        record: &mut F,
    ) -> bool {
        let requests = [
            requests.lane0,
            requests.lane1,
            requests.lane2,
            requests.lane3,
        ];
        let mut ffe_changed = false;
        for (lane, request) in requests.into_iter().enumerate() {
            let in_use = lane < self.count;
            match request {
                // An undefined value is ignored, as the Xilinx and Intel drivers do: a
                // lane in use keeps its pattern and level (see `record_undefined`). Lane 3
                // at a 3-lane rate is not in use.
                LtpReq::Reserved(_) => {}
                _ if !in_use => {}
                // Without FLT_no_timeout the lane keeps its previous pattern, as the
                // Xilinx driver does (spec Table 6-32, LTP3 row).
                LtpReq::NyquistClock if !no_timeout => {}
                LtpReq::FfeChange => {
                    // Held at the maximum once reached.
                    if self.levels[lane] < max_level {
                        self.levels[lane] += 1;
                        ffe_changed = true;
                        record(TrainingEvent::FfeRaised {
                            // At most 4 lanes and level 7: both fit in a u8.
                            lane: lane as u8,
                            level: self.levels[lane],
                        });
                    }
                }
                // 0x0 (and a 0xF on only some lanes) keeps the lane's pattern and level.
                request => {
                    if let Some(pattern) = request.pattern() {
                        self.patterns[lane] = Some(pattern);
                    }
                }
            }
        }
        ffe_changed
    }

    /// Records each undefined request (0x9–0xD), on every lane, in use or not, whatever
    /// the round leads to.
    fn record_undefined<F: FnMut(TrainingEvent)>(&self, requests: LtpRequests, record: &mut F) {
        let requests = [
            requests.lane0,
            requests.lane1,
            requests.lane2,
            requests.lane3,
        ];
        for (lane, request) in requests.into_iter().enumerate() {
            if let LtpReq::Reserved(value) = request {
                record(TrainingEvent::UndefinedLtpRequest {
                    // At most 4 lanes: fits in a u8.
                    lane: lane as u8,
                    value,
                    in_use: lane < self.count,
                });
            }
        }
    }

    /// Whether every lane in use made the given request.
    fn all(&self, requests: LtpRequests, request: LtpReq) -> bool {
        [
            requests.lane0,
            requests.lane1,
            requests.lane2,
            requests.lane3,
        ][..self.count]
            .iter()
            .all(|r| *r == request)
    }

    fn patterns(&self) -> LanePatterns {
        let lane = |i: usize| {
            if i < self.count {
                self.patterns[i]
            } else {
                None
            }
        };
        LanePatterns {
            lane0: lane(0),
            lane1: lane(1),
            lane2: lane(2),
            lane3: lane(3),
        }
    }

    pub(crate) fn eq_params(&self) -> EqParams {
        let mut params = EqParams::new();
        params.lane0 = lane_eq(self.levels[0]);
        params.lane1 = lane_eq(self.levels[1]);
        params.lane2 = lane_eq(self.levels[2]);
        params.lane3 = (self.count == 4).then(|| lane_eq(self.levels[3]));
        params
    }
}

/// The link training states.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum State {
    /// LTS:2.
    Prepare,
    /// LTS:3.
    Train,
    /// LTS:P.
    Pass,
    /// LTS:4.
    LowerRate,
    /// Training passed and the sink set `FRL_start`.
    Success,
    /// The attempt ends without a link.
    Fallback(FallbackReason),
    /// The `FLT_no_timeout` poll cap ran out: the link is left as it is.
    Hold,
}

/// How the states end, before LTS:L.
enum End {
    Success(HdmiForumFrl),
    Fallback(FallbackReason),
    Hold(HdmiForumFrl),
}

/// The state of one call to `train`.
#[derive(Debug)]
struct Attempt {
    rate: HdmiForumFrl,
    lanes: Lanes,
    /// The sink's `FLT_no_timeout`, as last read.
    no_timeout: bool,
    /// Returns from LTS:P to LTS:3 so far.
    retrains: u32,
}

impl Attempt {
    /// The poll limit for LTS:2, LTS:3 and LTS:P: `normal`, or the cap while the sink
    /// sets `FLT_no_timeout`.
    fn limit(&self, config: &TrainingConfig, normal: u32) -> u32 {
        if self.no_timeout {
            config.no_timeout_poll_cap
        } else {
            normal
        }
    }

    /// A state's poll limit ran out after `polls` polls. Normally the attempt falls back
    /// for `reason`, recording `timeout`. Under `FLT_no_timeout` the cap ran out instead:
    /// the test equipment is in control, so the attempt holds the link as it is rather
    /// than leaving FRL on its own timer, as the Xilinx and AMD drivers do.
    fn timed_out<F: FnMut(TrainingEvent)>(
        &self,
        polls: u32,
        timeout: TrainingEvent,
        reason: FallbackReason,
        record: &mut F,
    ) -> State {
        if self.no_timeout {
            record(TrainingEvent::NoTimeoutCapReached { polls });
            State::Hold
        } else {
            record(timeout);
            State::Fallback(reason)
        }
    }
}

/// The state machine, borrowing the I/O for one attempt.
struct Machine<'a, Io> {
    io: &'a mut Io,
}

impl<Io: TrainingIo> Machine<'_, Io> {
    /// One attempt: the states, then LTS:L on a fallback or an error. `record` is called
    /// with each [`TrainingEvent`] as it occurs.
    async fn run<F: FnMut(TrainingEvent)>(
        &mut self,
        rates: &[HdmiForumFrl],
        config: &TrainingConfig,
        record: &mut F,
    ) -> Result<TrainingOutcome, Error<Io>> {
        let Some((&rate, lower)) = rates.split_first() else {
            return Ok(TrainingOutcome::FallbackRequired {
                reason: FallbackReason::RatesExhausted,
            });
        };
        match self.states(rate, lower, config, record).await {
            Ok(End::Success(achieved_rate)) => Ok(TrainingOutcome::Success { achieved_rate }),
            // Under FLT_no_timeout the test equipment is in control: no LTS:L.
            Ok(End::Hold(rate)) => Ok(TrainingOutcome::NoTimeoutHold { rate }),
            Ok(End::Fallback(reason)) => match self.exit_to_tmds(record).await {
                Ok(()) => Ok(TrainingOutcome::FallbackRequired { reason }),
                Err(error) => Err(TrainingError::ExitFailed { reason, error }),
            },
            Err(fault) => {
                let exit = if config.exit_to_tmds_on_error {
                    match self.exit_to_tmds(record).await {
                        Ok(()) => TmdsExit::Exited,
                        Err(error) => TmdsExit::Failed(error),
                    }
                } else {
                    TmdsExit::Skipped
                };
                Err(match fault {
                    Fault::Scdc(error) => TrainingError::Scdc { error, exit },
                    Fault::Phy(error) => TrainingError::Phy { error, exit },
                })
            }
        }
    }

    /// LTS:2 → LTS:3 → LTS:P, with LTS:4, from `rate` down through `lower`. Returns the
    /// rate trained at, or why the attempt falls back.
    async fn states<F: FnMut(TrainingEvent)>(
        &mut self,
        rate: HdmiForumFrl,
        lower: &[HdmiForumFrl],
        config: &TrainingConfig,
        record: &mut F,
    ) -> Result<End, Failure<Io>> {
        let mut rates = lower.iter().copied();
        let mut attempt = Attempt {
            rate,
            lanes: Lanes::new(rate),
            no_timeout: false,
            retrains: 0,
        };
        let mut state = State::Prepare;
        loop {
            state = match state {
                State::Prepare => self.prepare(&mut attempt, config, record).await?,
                State::Train => self.train_lanes(&mut attempt, config, record).await?,
                State::Pass => self.pass(&mut attempt, config, record).await?,
                State::LowerRate => match rates.next() {
                    Some(next) => self.lower_rate(&mut attempt, next, config, record).await?,
                    None => {
                        record(TrainingEvent::RatesExhausted);
                        State::Fallback(FallbackReason::RatesExhausted)
                    }
                },
                State::Success => return Ok(End::Success(attempt.rate)),
                State::Fallback(reason) => return Ok(End::Fallback(reason)),
                State::Hold => return Ok(End::Hold(attempt.rate)),
            };
        }
    }

    /// LTS:2: wait for the sink, configure the PHY and write `Config_0` and `Config_1`.
    async fn prepare<F: FnMut(TrainingEvent)>(
        &mut self,
        attempt: &mut Attempt,
        config: &TrainingConfig,
        record: &mut F,
    ) -> Result<State, Failure<Io>> {
        // Read on every attempt, not only when `Source_Test_Update` says it changed: a
        // tester sets `FLT_no_timeout` and leaves it set, and the flag was cleared by an
        // earlier attempt. The Xilinx and AMD drivers read it unconditionally too.
        let update = self.read_update_flags().await?.source_test_update;
        self.read_source_test(attempt, update, record).await?;

        let limit = attempt.limit(config, config.flt_ready_polls);
        match self.poll_flt_ready(limit).await? {
            Some(polls) => record(TrainingEvent::FltReady { after_polls: polls }),
            None => {
                return Ok(attempt.timed_out(
                    limit,
                    TrainingEvent::FltReadyTimeout { polls: limit },
                    FallbackReason::FltReadyTimeout,
                    record,
                ));
            }
        }

        self.clear(FLT_UPDATE).await?;
        self.adjust_equalization(attempt.lanes.eq_params()).await?;

        let count = attempt.lanes.count;
        self.io
            .set_frl_rate(attempt.rate)
            .await
            .map_err(Fault::Phy)?;
        self.send_ltp(uniform(count, Some(LtpPattern::NyquistClock)))
            .await?;

        self.send_ltp(uniform(count, None)).await?;
        self.set_frl_output(FrlOutput::GapOnly).await?;

        self.io
            .write_config_0_defaults()
            .await
            .map_err(Fault::Scdc)?;
        self.write_rate(attempt.rate, config, record).await?;
        Ok(State::Train)
    }

    /// LTS:3: follow the sink's per-lane requests until it passes the lanes, asks for a
    /// lower rate, or the poll limit runs out.
    async fn train_lanes<F: FnMut(TrainingEvent)>(
        &mut self,
        attempt: &mut Attempt,
        config: &TrainingConfig,
        record: &mut F,
    ) -> Result<State, Failure<Io>> {
        let max_level = config.ffe_levels.limited_to(attempt.rate).value();
        let mut polls = 0;
        loop {
            let limit = attempt.limit(config, config.ltp_polls);
            if polls >= limit {
                return Ok(attempt.timed_out(
                    limit,
                    TrainingEvent::TrainingTimeout { polls: limit },
                    FallbackReason::TrainingTimeout,
                    record,
                ));
            }
            let flags = self.read_update_flags().await?;
            polls += 1;
            if !flags.flt_update {
                continue;
            }
            if flags.source_test_update {
                self.read_source_test(attempt, true, record).await?;
            }

            let requests = self.io.read_ltp_requests().await.map_err(Fault::Scdc)?;
            let passed = attempt.lanes.all(requests, LtpReq::None);
            if !passed {
                record(TrainingEvent::LtpRequested { requests });
            }
            attempt.lanes.record_undefined(requests, record);
            if passed {
                record(TrainingEvent::TrainingPassed { after_polls: polls });
                return Ok(State::Pass);
            }
            if attempt.lanes.all(requests, LtpReq::RateChange) {
                return Ok(State::LowerRate);
            }

            let ffe_changed = attempt
                .lanes
                .apply(requests, attempt.no_timeout, max_level, record);
            self.send_ltp(attempt.lanes.patterns()).await?;
            if ffe_changed {
                self.adjust_equalization(attempt.lanes.eq_params()).await?;
            }
            self.clear(FLT_UPDATE).await?;
        }
    }

    /// LTS:P: send gap characters until the sink sets `FRL_start`, or asks to retrain.
    /// A retrain returns to LTS:3 while `max_retrains` allows; the next one falls back.
    async fn pass<F: FnMut(TrainingEvent)>(
        &mut self,
        attempt: &mut Attempt,
        config: &TrainingConfig,
        record: &mut F,
    ) -> Result<State, Failure<Io>> {
        // The lanes always hold what the PHY was last told: stopping the patterns here
        // means a retrain starts LTS:3 from no pattern, as the PHY does. The TxFFE
        // levels are not touched, so they carry over.
        attempt.lanes.stop_patterns();
        self.send_ltp(attempt.lanes.patterns()).await?;
        self.set_frl_output(FrlOutput::GapOnly).await?;
        self.clear(FLT_UPDATE).await?;

        let limit = attempt.limit(config, config.frl_start_polls);
        for polls in 1..=limit {
            let flags = self.read_update_flags().await?;
            if flags.frl_start {
                self.clear(FRL_START).await?;
                record(TrainingEvent::FrlStart { after_polls: polls });
                return Ok(State::Success);
            }
            if flags.flt_update {
                if attempt.retrains >= config.max_retrains {
                    record(TrainingEvent::RetrainsExhausted {
                        retrains: attempt.retrains,
                    });
                    return Ok(State::Fallback(FallbackReason::RetrainsExhausted));
                }
                attempt.retrains += 1;
                record(TrainingEvent::RetrainRequested);
                return Ok(State::Train);
            }
        }
        Ok(attempt.timed_out(
            limit,
            TrainingEvent::FrlStartTimeout { polls: limit },
            FallbackReason::FrlStartTimeout,
            record,
        ))
    }

    /// LTS:4: continue training at `rate`, the next one in the list. The sink stays in
    /// FRL; `FLT_ready` is not awaited again.
    async fn lower_rate<F: FnMut(TrainingEvent)>(
        &mut self,
        attempt: &mut Attempt,
        rate: HdmiForumFrl,
        config: &TrainingConfig,
        record: &mut F,
    ) -> Result<State, Failure<Io>> {
        self.send_ltp(LanePatterns::default()).await?;
        record(TrainingEvent::RateLowered {
            from: attempt.rate,
            to: rate,
        });

        attempt.rate = rate;
        attempt.lanes = Lanes::new(rate);
        self.adjust_equalization(attempt.lanes.eq_params()).await?;
        self.io.set_frl_rate(rate).await.map_err(Fault::Phy)?;
        self.clear(FLT_UPDATE).await?;
        self.write_rate(rate, config, record).await?;
        Ok(State::Train)
    }

    /// LTS:L: leave both ends in TMDS. Stops the training patterns, returns the PHY to
    /// TMDS, turns FRL off in `Config_1` and clears `FLT_update` if it is set, so a failed
    /// attempt never leaves the sink configured for a rate the source is not driving.
    ///
    /// Every step is attempted even when an earlier one fails, so a PHY error cannot keep
    /// the sink in FRL, nor an SCDC error the PHY. On failure, returns each end's first
    /// error.
    async fn exit_to_tmds<F: FnMut(TrainingEvent)>(
        &mut self,
        record: &mut F,
    ) -> Result<(), ExitError<Io::ScdcError, Io::PhyError>> {
        let patterns = self.io.send_ltp(LanePatterns::default()).await;
        let phy_rate = self.io.set_frl_rate(HdmiForumFrl::NotSupported).await;
        let config = self
            .io
            .write_frl_config(FrlConfig {
                rate: HdmiForumFrl::NotSupported,
                ffe_levels: FfeLevels::default(),
            })
            .await;
        let flt_update = match self.io.read_update_flags().await {
            Ok(flags) if flags.flt_update => self.io.clear_update_flags(FLT_UPDATE).await,
            Ok(_) => Ok(()),
            Err(e) => Err(e),
        };
        let phy = patterns.and(phy_rate).err();
        let scdc = config.and(flt_update).err();
        if phy.is_none() && scdc.is_none() {
            record(TrainingEvent::ExitedToTmds);
            return Ok(());
        }
        record(TrainingEvent::ExitToTmdsFailed {
            scdc: scdc.is_some(),
            phy: phy.is_some(),
        });
        Err(ExitError { scdc, phy })
    }

    /// Writes `Config_1` with `rate` and the FFE levels limited for it.
    async fn write_rate<F: FnMut(TrainingEvent)>(
        &mut self,
        rate: HdmiForumFrl,
        config: &TrainingConfig,
        record: &mut F,
    ) -> Result<(), Failure<Io>> {
        let ffe_levels = config.ffe_levels.limited_to(rate);
        self.io
            .write_frl_config(FrlConfig { rate, ffe_levels })
            .await
            .map_err(Fault::Scdc)?;
        record(TrainingEvent::RateConfigured { rate, ffe_levels });
        Ok(())
    }

    /// Polls `FLT_ready` up to `limit` times. Returns the number of polls it took to
    /// assert, or `None` if it did not.
    async fn poll_flt_ready(&mut self, limit: u32) -> Result<Option<u32>, Failure<Io>> {
        for polls in 1..=limit {
            if self.io.read_flt_ready().await.map_err(Fault::Scdc)? {
                return Ok(Some(polls));
            }
        }
        Ok(None)
    }

    /// Reads `Source_Test_Configuration` into the attempt, and clears `Source_Test_Update`
    /// if `update` says it is set.
    async fn read_source_test<F: FnMut(TrainingEvent)>(
        &mut self,
        attempt: &mut Attempt,
        update: bool,
        record: &mut F,
    ) -> Result<(), Failure<Io>> {
        let source_test = self
            .io
            .read_source_test_config()
            .await
            .map_err(Fault::Scdc)?;
        attempt.no_timeout = source_test.flt_no_timeout;
        record(TrainingEvent::SourceTestConfigRead {
            flt_no_timeout: source_test.flt_no_timeout,
        });
        if update {
            self.clear(SOURCE_TEST_UPDATE).await?;
        }
        Ok(())
    }

    async fn read_update_flags(&mut self) -> Result<UpdateFlags, Failure<Io>> {
        self.io.read_update_flags().await.map_err(Fault::Scdc)
    }

    async fn clear(&mut self, flags: UpdateFlags) -> Result<(), Failure<Io>> {
        self.io.clear_update_flags(flags).await.map_err(Fault::Scdc)
    }

    async fn send_ltp(&mut self, patterns: LanePatterns) -> Result<(), Failure<Io>> {
        self.io.send_ltp(patterns).await.map_err(Fault::Phy)
    }

    async fn set_frl_output(&mut self, output: FrlOutput) -> Result<(), Failure<Io>> {
        self.io.set_frl_output(output).await.map_err(Fault::Phy)
    }

    async fn adjust_equalization(&mut self, params: EqParams) -> Result<(), Failure<Io>> {
        self.io
            .adjust_equalization(params)
            .await
            .map_err(Fault::Phy)
    }
}
