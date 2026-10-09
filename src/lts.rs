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
use crate::training::{FallbackReason, TrainingConfig, TrainingError, TrainingOutcome};
use crate::types::{FfeLevels, FrlConfig, LtpReq, LtpRequests, SourceTestConfig, UpdateFlags};

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
pub async fn run<Io: TrainingIo, F: FnMut(TrainingEvent)>(
    io: &mut Io,
    rates: &[HdmiForumFrl],
    config: &TrainingConfig,
    record: &mut F,
) -> Result<TrainingOutcome, Error<Io>> {
    Machine { io }.run(rates, config, record).await
}

type Error<Io> = TrainingError<<Io as TrainingIo>::ScdcError, <Io as TrainingIo>::PhyError>;

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

/// Each lane's current pattern and TxFFE level, held by plumbob through LTS:3. The PHY
/// applies exactly what it is given.
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
        for (lane, request) in requests.into_iter().enumerate().take(self.count) {
            match request {
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
    /// The poll limit for LTS:2 and LTS:3: `normal`, or the cap while the sink sets
    /// `FLT_no_timeout`.
    fn limit(&self, config: &TrainingConfig, normal: u32) -> u32 {
        if self.no_timeout {
            config.no_timeout_poll_cap
        } else {
            normal
        }
    }
}

/// The state machine, borrowing the I/O for one attempt.
struct Machine<'a, Io> {
    io: &'a mut Io,
}

impl<Io: TrainingIo> Machine<'_, Io> {
    /// The state machine. `record` is called with each [`TrainingEvent`] as it occurs.
    async fn run<F: FnMut(TrainingEvent)>(
        &mut self,
        rates: &[HdmiForumFrl],
        config: &TrainingConfig,
        record: &mut F,
    ) -> Result<TrainingOutcome, Error<Io>> {
        let mut rates = rates.iter().copied();
        let Some(rate) = rates.next() else {
            return Ok(TrainingOutcome::FallbackRequired {
                reason: FallbackReason::RatesExhausted,
            });
        };
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
                State::Success => {
                    return Ok(TrainingOutcome::Success {
                        achieved_rate: attempt.rate,
                    });
                }
                State::Fallback(reason) => {
                    self.exit_to_tmds(record).await?;
                    return Ok(TrainingOutcome::FallbackRequired { reason });
                }
            };
        }
    }

    /// LTS:2: wait for the sink, configure the PHY and write `Config_0` and `Config_1`.
    async fn prepare<F: FnMut(TrainingEvent)>(
        &mut self,
        attempt: &mut Attempt,
        config: &TrainingConfig,
        record: &mut F,
    ) -> Result<State, Error<Io>> {
        if self.read_update_flags().await?.source_test_update {
            self.read_source_test(attempt, record).await?;
        }

        let limit = attempt.limit(config, config.flt_ready_polls);
        match self.poll_flt_ready(limit).await? {
            Some(polls) => record(TrainingEvent::FltReady { after_polls: polls }),
            None => {
                record(TrainingEvent::FltReadyTimeout { polls: limit });
                return Ok(State::Fallback(FallbackReason::FltReadyTimeout));
            }
        }

        self.clear(FLT_UPDATE).await?;
        self.adjust_equalization(attempt.lanes.eq_params()).await?;

        let count = attempt.lanes.count;
        self.io
            .set_frl_rate(attempt.rate)
            .await
            .map_err(TrainingError::Phy)?;
        self.send_ltp(uniform(count, Some(LtpPattern::NyquistClock)))
            .await?;

        self.send_ltp(uniform(count, None)).await?;
        self.set_frl_output(FrlOutput::GapOnly).await?;

        self.io
            .write_config_0_defaults()
            .await
            .map_err(TrainingError::Scdc)?;
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
    ) -> Result<State, Error<Io>> {
        let max_level = config.ffe_levels.limited_to(attempt.rate).value();
        let mut polls = 0;
        loop {
            let limit = attempt.limit(config, config.ltp_polls);
            if polls >= limit {
                record(TrainingEvent::TrainingTimeout { polls: limit });
                return Ok(State::Fallback(FallbackReason::TrainingTimeout));
            }
            let flags = self.read_update_flags().await?;
            polls += 1;
            if !flags.flt_update {
                continue;
            }
            if flags.source_test_update {
                self.read_source_test(attempt, record).await?;
            }

            let requests = self
                .io
                .read_ltp_requests()
                .await
                .map_err(TrainingError::Scdc)?;
            if attempt.lanes.all(requests, LtpReq::None) {
                record(TrainingEvent::TrainingPassed { after_polls: polls });
                return Ok(State::Pass);
            }
            record(TrainingEvent::LtpRequested { requests });
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
    ) -> Result<State, Error<Io>> {
        self.send_ltp(uniform(attempt.lanes.count, None)).await?;
        self.set_frl_output(FrlOutput::GapOnly).await?;
        self.clear(FLT_UPDATE).await?;

        for polls in 1..=config.frl_start_polls {
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
        record(TrainingEvent::FrlStartTimeout {
            polls: config.frl_start_polls,
        });
        Ok(State::Fallback(FallbackReason::FrlStartTimeout))
    }

    /// LTS:4: continue training at `rate`, the next one in the list. The sink stays in
    /// FRL; `FLT_ready` is not awaited again.
    async fn lower_rate<F: FnMut(TrainingEvent)>(
        &mut self,
        attempt: &mut Attempt,
        rate: HdmiForumFrl,
        config: &TrainingConfig,
        record: &mut F,
    ) -> Result<State, Error<Io>> {
        self.send_ltp(LanePatterns::default()).await?;
        record(TrainingEvent::RateLowered {
            from: attempt.rate,
            to: rate,
        });

        attempt.rate = rate;
        attempt.lanes = Lanes::new(rate);
        self.adjust_equalization(attempt.lanes.eq_params()).await?;
        self.io
            .set_frl_rate(rate)
            .await
            .map_err(TrainingError::Phy)?;
        self.clear(FLT_UPDATE).await?;
        self.write_rate(rate, config, record).await?;
        Ok(State::Train)
    }

    /// LTS:L: leave both ends in TMDS. Stops the training patterns, returns the PHY to
    /// TMDS, turns FRL off in `Config_1` and clears `FLT_update` if it is set, so a failed
    /// attempt never leaves the sink configured for a rate the source is not driving.
    async fn exit_to_tmds<F: FnMut(TrainingEvent)>(
        &mut self,
        record: &mut F,
    ) -> Result<(), Error<Io>> {
        self.send_ltp(LanePatterns::default()).await?;
        self.io
            .set_frl_rate(HdmiForumFrl::NotSupported)
            .await
            .map_err(TrainingError::Phy)?;
        self.io
            .write_frl_config(FrlConfig {
                rate: HdmiForumFrl::NotSupported,
                ffe_levels: FfeLevels::default(),
            })
            .await
            .map_err(TrainingError::Scdc)?;
        if self.read_update_flags().await?.flt_update {
            self.clear(FLT_UPDATE).await?;
        }
        record(TrainingEvent::ExitedToTmds);
        Ok(())
    }

    /// Writes `Config_1` with `rate` and the FFE levels limited for it.
    async fn write_rate<F: FnMut(TrainingEvent)>(
        &mut self,
        rate: HdmiForumFrl,
        config: &TrainingConfig,
        record: &mut F,
    ) -> Result<(), Error<Io>> {
        let ffe_levels = config.ffe_levels.limited_to(rate);
        self.io
            .write_frl_config(FrlConfig { rate, ffe_levels })
            .await
            .map_err(TrainingError::Scdc)?;
        record(TrainingEvent::RateConfigured { rate, ffe_levels });
        Ok(())
    }

    /// Polls `FLT_ready` up to `limit` times. Returns the number of polls it took to
    /// assert, or `None` if it did not.
    async fn poll_flt_ready(&mut self, limit: u32) -> Result<Option<u32>, Error<Io>> {
        for polls in 1..=limit {
            if self
                .io
                .read_flt_ready()
                .await
                .map_err(TrainingError::Scdc)?
            {
                return Ok(Some(polls));
            }
        }
        Ok(None)
    }

    /// Reads `Source_Test_Configuration` into the attempt and clears
    /// `Source_Test_Update`.
    async fn read_source_test<F: FnMut(TrainingEvent)>(
        &mut self,
        attempt: &mut Attempt,
        record: &mut F,
    ) -> Result<(), Error<Io>> {
        let source_test = self
            .io
            .read_source_test_config()
            .await
            .map_err(TrainingError::Scdc)?;
        attempt.no_timeout = source_test.flt_no_timeout;
        record(TrainingEvent::SourceTestConfigRead {
            flt_no_timeout: source_test.flt_no_timeout,
        });
        self.clear(SOURCE_TEST_UPDATE).await
    }

    async fn read_update_flags(&mut self) -> Result<UpdateFlags, Error<Io>> {
        self.io
            .read_update_flags()
            .await
            .map_err(TrainingError::Scdc)
    }

    async fn clear(&mut self, flags: UpdateFlags) -> Result<(), Error<Io>> {
        self.io
            .clear_update_flags(flags)
            .await
            .map_err(TrainingError::Scdc)
    }

    async fn send_ltp(&mut self, patterns: LanePatterns) -> Result<(), Error<Io>> {
        self.io.send_ltp(patterns).await.map_err(TrainingError::Phy)
    }

    async fn set_frl_output(&mut self, output: FrlOutput) -> Result<(), Error<Io>> {
        self.io
            .set_frl_output(output)
            .await
            .map_err(TrainingError::Phy)
    }

    async fn adjust_equalization(&mut self, params: EqParams) -> Result<(), Error<Io>> {
        self.io
            .adjust_equalization(params)
            .await
            .map_err(TrainingError::Phy)
    }
}
