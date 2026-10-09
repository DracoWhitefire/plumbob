use display_types::cea861::hdmi_forum::HdmiForumFrl;
use hdmi_hal::phy::{
    EqParams, FrlOutput, HdmiPhy, LaneEqParams, LanePatterns, LtpPattern, TxFfeLevel,
};

use super::scdc::ScdcClient;
use super::types::{FfeLevels, FrlConfig, LtpReq, LtpRequests, UpdateFlags};
use crate::training::TrainingError;

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
    pub ffe_levels: FfeLevels,
    /// Poll limit for `FLT_ready` in LTS:2. Default 50 (100 ms at 2 ms per poll).
    pub flt_ready_polls: u32,
    /// Poll limit for LTS:3. Default 100 (200 ms at 2 ms per poll).
    pub ltp_polls: u32,
    /// Poll limit for `FRL_start` in LTS:P. Default 125 (250 ms at 2 ms per poll).
    pub frl_start_polls: u32,
    /// Hard cap on the LTS:2 and LTS:3 polls while the sink sets `FLT_no_timeout`.
    /// Default 100 000.
    pub no_timeout_poll_cap: u32,
}

impl Default for TrainingConfig {
    fn default() -> Self {
        Self {
            ffe_levels: FfeLevels::default(),
            flt_ready_polls: 50,
            ltp_polls: 100,
            frl_start_polls: 125,
            no_timeout_poll_cap: 100_000,
        }
    }
}

const FLT_UPDATE: UpdateFlags = UpdateFlags {
    source_test_update: false,
    frl_start: false,
    flt_update: true,
};

const FRL_START: UpdateFlags = UpdateFlags {
    source_test_update: false,
    frl_start: true,
    flt_update: false,
};

const SOURCE_TEST_UPDATE: UpdateFlags = UpdateFlags {
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
fn uniform(count: usize, pattern: Option<LtpPattern>) -> LanePatterns {
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
struct Lanes {
    count: usize,
    patterns: [Option<LtpPattern>; 4],
    levels: [u8; 4],
}

impl Lanes {
    /// Every lane with no pattern and TxFFE level 0.
    fn new(rate: HdmiForumFrl) -> Self {
        Self {
            count: lane_count(rate),
            patterns: [None; 4],
            levels: [0; 4],
        }
    }

    /// Updates each lane from the sink's request for it. Returns whether any TxFFE level
    /// changed.
    fn apply(&mut self, requests: LtpRequests, no_timeout: bool, max_level: u8) -> bool {
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

    fn eq_params(&self) -> EqParams {
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

    /// Trains at a single rate: `train(&[rate], config)`.
    pub fn train_at_rate(
        &mut self,
        rate: HdmiForumFrl,
        config: &TrainingConfig,
    ) -> Result<TrainingOutcome, Error<C, P>> {
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
    ) -> Result<TrainingOutcome, Error<C, P>> {
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
        };
        let mut state = State::Prepare;
        loop {
            state = match state {
                State::Prepare => self.prepare(&mut attempt, config)?,
                State::Train => self.train_lanes(&mut attempt, config)?,
                State::Pass => self.pass(&attempt, config)?,
                State::LowerRate => match rates.next() {
                    Some(next) => self.lower_rate(&mut attempt, next, config)?,
                    None => State::Fallback(FallbackReason::RatesExhausted),
                },
                State::Success => {
                    return Ok(TrainingOutcome::Success {
                        achieved_rate: attempt.rate,
                    });
                }
                State::Fallback(reason) => {
                    self.exit_to_tmds()?;
                    return Ok(TrainingOutcome::FallbackRequired { reason });
                }
            };
        }
    }

    /// LTS:2: wait for the sink, configure the PHY and write `Config_0` and `Config_1`.
    fn prepare(
        &mut self,
        attempt: &mut Attempt,
        config: &TrainingConfig,
    ) -> Result<State, Error<C, P>> {
        if self.read_update_flags()?.source_test_update {
            self.read_source_test(attempt)?;
        }

        let limit = attempt.limit(config, config.flt_ready_polls);
        if !self.poll_flt_ready(limit)? {
            return Ok(State::Fallback(FallbackReason::FltReadyTimeout));
        }

        self.clear(FLT_UPDATE)?;
        self.adjust_equalization(attempt.lanes.eq_params())?;

        let count = attempt.lanes.count;
        self.phy
            .set_frl_rate(attempt.rate)
            .map_err(TrainingError::Phy)?;
        self.send_ltp(uniform(count, Some(LtpPattern::NyquistClock)))?;

        self.send_ltp(uniform(count, None))?;
        self.set_frl_output(FrlOutput::GapOnly)?;

        self.scdc
            .write_config_0_defaults()
            .map_err(TrainingError::Scdc)?;
        self.scdc
            .write_frl_config(FrlConfig {
                rate: attempt.rate,
                ffe_levels: config.ffe_levels.limited_to(attempt.rate),
            })
            .map_err(TrainingError::Scdc)?;
        Ok(State::Train)
    }

    /// LTS:3: follow the sink's per-lane requests until it passes the lanes, asks for a
    /// lower rate, or the poll limit runs out.
    fn train_lanes(
        &mut self,
        attempt: &mut Attempt,
        config: &TrainingConfig,
    ) -> Result<State, Error<C, P>> {
        let max_level = config.ffe_levels.limited_to(attempt.rate).value();
        let mut polls = 0;
        loop {
            if polls >= attempt.limit(config, config.ltp_polls) {
                return Ok(State::Fallback(FallbackReason::TrainingTimeout));
            }
            let flags = self.read_update_flags()?;
            polls += 1;
            if !flags.flt_update {
                continue;
            }
            if flags.source_test_update {
                self.read_source_test(attempt)?;
            }

            let requests = self.scdc.read_ltp_requests().map_err(TrainingError::Scdc)?;
            if attempt.lanes.all(requests, LtpReq::None) {
                return Ok(State::Pass);
            }
            if attempt.lanes.all(requests, LtpReq::RateChange) {
                return Ok(State::LowerRate);
            }

            let ffe_changed = attempt.lanes.apply(requests, attempt.no_timeout, max_level);
            self.send_ltp(attempt.lanes.patterns())?;
            if ffe_changed {
                self.adjust_equalization(attempt.lanes.eq_params())?;
            }
            self.clear(FLT_UPDATE)?;
        }
    }

    /// LTS:P: send gap characters until the sink sets `FRL_start`, or asks to retrain.
    fn pass(&mut self, attempt: &Attempt, config: &TrainingConfig) -> Result<State, Error<C, P>> {
        self.send_ltp(uniform(attempt.lanes.count, None))?;
        self.set_frl_output(FrlOutput::GapOnly)?;
        self.clear(FLT_UPDATE)?;

        for _ in 0..config.frl_start_polls {
            let flags = self.read_update_flags()?;
            if flags.frl_start {
                self.clear(FRL_START)?;
                return Ok(State::Success);
            }
            if flags.flt_update {
                return Ok(State::Train);
            }
        }
        Ok(State::Fallback(FallbackReason::FrlStartTimeout))
    }

    /// LTS:4: continue training at `rate`, the next one in the list. The sink stays in
    /// FRL; `FLT_ready` is not awaited again.
    fn lower_rate(
        &mut self,
        attempt: &mut Attempt,
        rate: HdmiForumFrl,
        config: &TrainingConfig,
    ) -> Result<State, Error<C, P>> {
        self.send_ltp(LanePatterns::default())?;

        attempt.rate = rate;
        attempt.lanes = Lanes::new(rate);
        self.adjust_equalization(attempt.lanes.eq_params())?;
        self.phy.set_frl_rate(rate).map_err(TrainingError::Phy)?;
        self.clear(FLT_UPDATE)?;
        self.scdc
            .write_frl_config(FrlConfig {
                rate,
                ffe_levels: config.ffe_levels.limited_to(rate),
            })
            .map_err(TrainingError::Scdc)?;
        Ok(State::Train)
    }

    /// LTS:L: leave both ends in TMDS. Stops the training patterns, returns the PHY to
    /// TMDS, turns FRL off in `Config_1` and clears `FLT_update` if it is set, so a failed
    /// attempt never leaves the sink configured for a rate the source is not driving.
    fn exit_to_tmds(&mut self) -> Result<(), Error<C, P>> {
        self.send_ltp(LanePatterns::default())?;
        self.phy
            .set_frl_rate(HdmiForumFrl::NotSupported)
            .map_err(TrainingError::Phy)?;
        self.scdc
            .write_frl_config(FrlConfig {
                rate: HdmiForumFrl::NotSupported,
                ffe_levels: FfeLevels::default(),
            })
            .map_err(TrainingError::Scdc)?;
        if self.read_update_flags()?.flt_update {
            self.clear(FLT_UPDATE)?;
        }
        Ok(())
    }

    /// Polls `FLT_ready` up to `limit` times. Returns whether it asserted.
    fn poll_flt_ready(&mut self, limit: u32) -> Result<bool, Error<C, P>> {
        for _ in 0..limit {
            if self.scdc.read_flt_ready().map_err(TrainingError::Scdc)? {
                return Ok(true);
            }
        }
        Ok(false)
    }

    /// Reads `Source_Test_Configuration` into the attempt and clears
    /// `Source_Test_Update`.
    fn read_source_test(&mut self, attempt: &mut Attempt) -> Result<(), Error<C, P>> {
        let source_test = self
            .scdc
            .read_source_test_config()
            .map_err(TrainingError::Scdc)?;
        attempt.no_timeout = source_test.flt_no_timeout;
        self.clear(SOURCE_TEST_UPDATE)
    }

    fn read_update_flags(&mut self) -> Result<UpdateFlags, Error<C, P>> {
        self.scdc.read_update_flags().map_err(TrainingError::Scdc)
    }

    fn clear(&mut self, flags: UpdateFlags) -> Result<(), Error<C, P>> {
        self.scdc
            .clear_update_flags(flags)
            .map_err(TrainingError::Scdc)
    }

    fn send_ltp(&mut self, patterns: LanePatterns) -> Result<(), Error<C, P>> {
        self.phy.send_ltp(patterns).map_err(TrainingError::Phy)
    }

    fn set_frl_output(&mut self, output: FrlOutput) -> Result<(), Error<C, P>> {
        self.phy.set_frl_output(output).map_err(TrainingError::Phy)
    }

    fn adjust_equalization(&mut self, params: EqParams) -> Result<(), Error<C, P>> {
        self.phy
            .adjust_equalization(params)
            .map_err(TrainingError::Phy)
    }
}

#[cfg(test)]
#[path = "trainer_tests.rs"]
mod tests;
