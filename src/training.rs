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
            let status = self.scdc.read_training_status().map_err(TrainingError::Scdc)?;
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
            |i| TrainingEvent::FltReadyReceived { after_iterations: i },
            |i| TrainingEvent::FltReadyTimeout { iterations_elapsed: i },
            record,
        )? {
            return Ok(outcome);
        }

        // Phase 3 — Initiation: poll until the sink asserts frl_start.
        if let Some(outcome) = self.poll_until(
            config.frl_start_timeout,
            |s| s.frl_start,
            |i| TrainingEvent::FrlStartReceived { after_iterations: i },
            |i| TrainingEvent::FrlStartTimeout { iterations_elapsed: i },
            record,
        )? {
            return Ok(outcome);
        }

        // Phase 4 — LTP loop: drive patterns until ltp_req reaches None.
        //
        // read_ced is called on each iteration; it will feed per-lane equalization
        // adjustments once LaneEqParams fields are defined in hdmi-hal.
        // LtpPatternRequested is emitted only on transitions, not every poll.
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
mod tests {
    extern crate std;
    use std::collections::VecDeque;

    use display_types::cea861::hdmi_forum::HdmiForumFrl;
    use hdmi_hal::phy::{EqParams, HdmiPhy, LtpPattern};

    use super::*;
    use crate::scdc::ScdcClient;
    use crate::types::{CedCounters, FrlConfig, LtpReq, TrainingStatus};

    // -------------------------------------------------------------------------
    // SimScdc — scripted SCDC client for state machine tests
    // -------------------------------------------------------------------------

    struct SimScdc {
        /// Scripted responses for `read_training_status`, consumed in order.
        statuses: VecDeque<Result<TrainingStatus, ()>>,
        fail_write_frl_config: bool,
        fail_read_ced: bool,
        /// The last config written via `write_frl_config`.
        written_config: Option<FrlConfig>,
        ced_calls: u32,
    }

    impl SimScdc {
        fn new() -> Self {
            Self {
                statuses: VecDeque::new(),
                fail_write_frl_config: false,
                fail_read_ced: false,
                written_config: None,
                ced_calls: 0,
            }
        }

        fn push(&mut self, status: TrainingStatus) {
            self.statuses.push_back(Ok(status));
        }

        fn push_err(&mut self) {
            self.statuses.push_back(Err(()));
        }
    }

    impl ScdcClient for SimScdc {
        type Error = ();

        fn write_frl_config(&mut self, config: FrlConfig) -> Result<(), ()> {
            if self.fail_write_frl_config {
                return Err(());
            }
            self.written_config = Some(config);
            Ok(())
        }

        fn read_training_status(&mut self) -> Result<TrainingStatus, ()> {
            self.statuses.pop_front().unwrap_or(Err(()))
        }

        fn read_ced(&mut self) -> Result<CedCounters, ()> {
            if self.fail_read_ced {
                return Err(());
            }
            self.ced_calls += 1;
            Ok(CedCounters {
                lane0: None,
                lane1: None,
                lane2: None,
                lane3: None,
            })
        }
    }

    // -------------------------------------------------------------------------
    // MockPhy — records calls and supports injectable errors
    // -------------------------------------------------------------------------

    struct MockPhy {
        fail_set_frl_rate: bool,
        fail_send_ltp: bool,
        frl_rate: Option<HdmiForumFrl>,
        last_ltp: Option<LtpPattern>,
    }

    impl MockPhy {
        fn new() -> Self {
            Self {
                fail_set_frl_rate: false,
                fail_send_ltp: false,
                frl_rate: None,
                last_ltp: None,
            }
        }
    }

    impl HdmiPhy for MockPhy {
        type Error = ();

        fn send_ltp(&mut self, pattern: LtpPattern) -> Result<(), ()> {
            if self.fail_send_ltp {
                return Err(());
            }
            self.last_ltp = Some(pattern);
            Ok(())
        }

        fn set_frl_rate(&mut self, rate: HdmiForumFrl) -> Result<(), ()> {
            if self.fail_set_frl_rate {
                return Err(());
            }
            self.frl_rate = Some(rate);
            Ok(())
        }

        fn adjust_equalization(&mut self, _params: EqParams) -> Result<(), ()> {
            Ok(())
        }

        fn set_scrambling(&mut self, _enabled: bool) -> Result<(), ()> {
            Ok(())
        }
    }

    // -------------------------------------------------------------------------
    // Status helpers and fixtures
    // -------------------------------------------------------------------------

    fn not_ready() -> TrainingStatus {
        TrainingStatus {
            flt_ready: false,
            frl_start: false,
            ltp_req: LtpReq::None,
        }
    }

    fn flt_ready() -> TrainingStatus {
        TrainingStatus {
            flt_ready: true,
            frl_start: false,
            ltp_req: LtpReq::None,
        }
    }

    fn frl_started() -> TrainingStatus {
        TrainingStatus {
            flt_ready: true,
            frl_start: true,
            ltp_req: LtpReq::None,
        }
    }

    fn ltp(pattern: LtpReq) -> TrainingStatus {
        TrainingStatus {
            flt_ready: true,
            frl_start: true,
            ltp_req: pattern,
        }
    }

    const RATE: HdmiForumFrl = HdmiForumFrl::Rate6Gbps4Lanes;

    fn cfg(flt: u32, frl: u32, ltp_t: u32) -> TrainingConfig {
        TrainingConfig {
            flt_ready_timeout: flt,
            frl_start_timeout: frl,
            ltp_timeout: ltp_t,
            ..TrainingConfig::default()
        }
    }

    // -------------------------------------------------------------------------
    // TrainingConfig defaults
    // -------------------------------------------------------------------------

    #[test]
    fn training_config_default_ffe_levels() {
        assert_eq!(TrainingConfig::default().ffe_levels, FfeLevels::Ffe0);
    }

    #[test]
    fn training_config_default_dsc_frl_max() {
        assert!(!TrainingConfig::default().dsc_frl_max);
    }

    #[test]
    fn training_config_default_timeouts() {
        let c = TrainingConfig::default();
        assert_eq!(c.flt_ready_timeout, 1000);
        assert_eq!(c.frl_start_timeout, 1000);
        assert_eq!(c.ltp_timeout, 1000);
    }

    // -------------------------------------------------------------------------
    // TrainingOutcome and TrainingError constructibility
    // -------------------------------------------------------------------------

    #[test]
    fn training_outcome_success_carries_rate() {
        let o = TrainingOutcome::Success {
            achieved_rate: HdmiForumFrl::Rate6Gbps4Lanes,
        };
        assert_eq!(
            o,
            TrainingOutcome::Success {
                achieved_rate: HdmiForumFrl::Rate6Gbps4Lanes
            }
        );
    }

    #[test]
    fn training_outcome_fallback_required() {
        assert_eq!(
            TrainingOutcome::FallbackRequired,
            TrainingOutcome::FallbackRequired
        );
    }

    #[test]
    fn training_outcome_variants_are_distinct() {
        assert_ne!(
            TrainingOutcome::Success {
                achieved_rate: HdmiForumFrl::Rate6Gbps4Lanes
            },
            TrainingOutcome::FallbackRequired,
        );
    }

    #[test]
    fn training_error_scdc_variant() {
        let e: TrainingError<u8, u8> = TrainingError::Scdc(42);
        assert_eq!(e, TrainingError::Scdc(42));
    }

    #[test]
    fn training_error_phy_variant() {
        let e: TrainingError<u8, u8> = TrainingError::Phy(7);
        assert_eq!(e, TrainingError::Phy(7));
    }

    #[test]
    fn training_error_variants_are_distinct() {
        let s: TrainingError<u8, u8> = TrainingError::Scdc(1);
        let p: TrainingError<u8, u8> = TrainingError::Phy(1);
        assert_ne!(s, p);
    }

    // -------------------------------------------------------------------------
    // State machine: normal paths
    // -------------------------------------------------------------------------

    #[test]
    fn successful_training() {
        let mut scdc = SimScdc::new();
        // Phase 2: 2 not-ready, then flt_ready (after_iterations = 2)
        scdc.push(not_ready());
        scdc.push(not_ready());
        scdc.push(flt_ready());
        // Phase 3: 1 not-started, then frl_start (after_iterations = 1)
        scdc.push(flt_ready());
        scdc.push(frl_started());
        // Phase 4: two pattern requests then success
        scdc.push(ltp(LtpReq::Lfsr0));
        scdc.push(ltp(LtpReq::Lfsr2));
        scdc.push(frl_started()); // ltp_req = None → success

        let outcome = FrlTrainer::new(scdc, MockPhy::new())
            .train_at_rate(RATE, &TrainingConfig::default())
            .unwrap();
        assert_eq!(
            outcome,
            TrainingOutcome::Success {
                achieved_rate: RATE
            }
        );
    }

    #[test]
    fn flt_ready_timeout() {
        let mut scdc = SimScdc::new();
        scdc.push(not_ready());
        scdc.push(not_ready());
        scdc.push(not_ready()); // i reaches flt_ready_timeout = 3

        let outcome = FrlTrainer::new(scdc, MockPhy::new())
            .train_at_rate(RATE, &cfg(3, 10, 10))
            .unwrap();
        assert_eq!(outcome, TrainingOutcome::FallbackRequired);
    }

    #[test]
    fn flt_ready_immediate() {
        let mut scdc = SimScdc::new();
        scdc.push(flt_ready()); // asserts on first read: i = 0
        scdc.push(frl_started());
        scdc.push(frl_started()); // phase 4: ltp_req = None

        let outcome = FrlTrainer::new(scdc, MockPhy::new())
            .train_at_rate(RATE, &TrainingConfig::default())
            .unwrap();
        assert_eq!(
            outcome,
            TrainingOutcome::Success {
                achieved_rate: RATE
            }
        );
    }

    #[test]
    fn frl_start_timeout() {
        let mut scdc = SimScdc::new();
        scdc.push(flt_ready()); // phase 2: immediate
        // Phase 3: i reaches frl_start_timeout = 3
        scdc.push(flt_ready());
        scdc.push(flt_ready());
        scdc.push(flt_ready());

        let outcome = FrlTrainer::new(scdc, MockPhy::new())
            .train_at_rate(RATE, &cfg(10, 3, 10))
            .unwrap();
        assert_eq!(outcome, TrainingOutcome::FallbackRequired);
    }

    #[test]
    fn frl_start_immediate() {
        let mut scdc = SimScdc::new();
        scdc.push(flt_ready()); // phase 2: immediate (i = 0)
        scdc.push(frl_started()); // phase 3: immediate (i = 0)
        scdc.push(frl_started()); // phase 4: ltp_req = None

        let outcome = FrlTrainer::new(scdc, MockPhy::new())
            .train_at_rate(RATE, &TrainingConfig::default())
            .unwrap();
        assert_eq!(
            outcome,
            TrainingOutcome::Success {
                achieved_rate: RATE
            }
        );
    }

    #[test]
    fn ltp_loop_timeout() {
        let mut scdc = SimScdc::new();
        scdc.push(flt_ready());
        scdc.push(frl_started());
        // Phase 4: i reaches ltp_timeout = 3
        scdc.push(ltp(LtpReq::Lfsr1));
        scdc.push(ltp(LtpReq::Lfsr1));
        scdc.push(ltp(LtpReq::Lfsr1));

        let outcome = FrlTrainer::new(scdc, MockPhy::new())
            .train_at_rate(RATE, &cfg(10, 10, 3))
            .unwrap();
        assert_eq!(outcome, TrainingOutcome::FallbackRequired);
    }

    #[test]
    fn ltp_success_on_first_read() {
        let mut scdc = SimScdc::new();
        scdc.push(flt_ready());
        scdc.push(frl_started());
        scdc.push(frl_started()); // ltp_req = None on the first LTP read

        let outcome = FrlTrainer::new(scdc, MockPhy::new())
            .train_at_rate(RATE, &TrainingConfig::default())
            .unwrap();
        assert_eq!(
            outcome,
            TrainingOutcome::Success {
                achieved_rate: RATE
            }
        );
    }

    #[test]
    fn ltp_all_lfsr_variants() {
        let mut scdc = SimScdc::new();
        scdc.push(flt_ready());
        scdc.push(frl_started());
        scdc.push(ltp(LtpReq::Lfsr0));
        scdc.push(ltp(LtpReq::Lfsr1));
        scdc.push(ltp(LtpReq::Lfsr2));
        scdc.push(ltp(LtpReq::Lfsr3));
        scdc.push(frl_started()); // ltp_req = None

        let outcome = FrlTrainer::new(scdc, MockPhy::new())
            .train_at_rate(RATE, &TrainingConfig::default())
            .unwrap();
        assert_eq!(
            outcome,
            TrainingOutcome::Success {
                achieved_rate: RATE
            }
        );
    }

    // -------------------------------------------------------------------------
    // Error propagation
    // -------------------------------------------------------------------------

    #[test]
    fn error_write_frl_config() {
        let mut scdc = SimScdc::new();
        scdc.fail_write_frl_config = true;

        let err = FrlTrainer::new(scdc, MockPhy::new())
            .train_at_rate(RATE, &TrainingConfig::default())
            .unwrap_err();
        assert_eq!(err, TrainingError::Scdc(()));
    }

    #[test]
    fn error_set_frl_rate() {
        let mut phy = MockPhy::new();
        phy.fail_set_frl_rate = true;

        let err = FrlTrainer::new(SimScdc::new(), phy)
            .train_at_rate(RATE, &TrainingConfig::default())
            .unwrap_err();
        assert_eq!(err, TrainingError::Phy(()));
    }

    #[test]
    fn error_read_training_status_phase2() {
        let mut scdc = SimScdc::new();
        scdc.push_err();

        let err = FrlTrainer::new(scdc, MockPhy::new())
            .train_at_rate(RATE, &TrainingConfig::default())
            .unwrap_err();
        assert_eq!(err, TrainingError::Scdc(()));
    }

    #[test]
    fn error_read_training_status_phase3() {
        let mut scdc = SimScdc::new();
        scdc.push(flt_ready());
        scdc.push_err();

        let err = FrlTrainer::new(scdc, MockPhy::new())
            .train_at_rate(RATE, &TrainingConfig::default())
            .unwrap_err();
        assert_eq!(err, TrainingError::Scdc(()));
    }

    #[test]
    fn error_read_training_status_phase4() {
        let mut scdc = SimScdc::new();
        scdc.push(flt_ready());
        scdc.push(frl_started());
        scdc.push_err();

        let err = FrlTrainer::new(scdc, MockPhy::new())
            .train_at_rate(RATE, &TrainingConfig::default())
            .unwrap_err();
        assert_eq!(err, TrainingError::Scdc(()));
    }

    #[test]
    fn error_read_ced() {
        let mut scdc = SimScdc::new();
        scdc.push(flt_ready());
        scdc.push(frl_started());
        scdc.push(ltp(LtpReq::Lfsr0)); // non-None → read_ced is called
        scdc.fail_read_ced = true;

        let err = FrlTrainer::new(scdc, MockPhy::new())
            .train_at_rate(RATE, &TrainingConfig::default())
            .unwrap_err();
        assert_eq!(err, TrainingError::Scdc(()));
    }

    #[test]
    fn error_send_ltp() {
        let mut scdc = SimScdc::new();
        scdc.push(flt_ready());
        scdc.push(frl_started());
        scdc.push(ltp(LtpReq::Lfsr0)); // non-None → send_ltp is called
        let mut phy = MockPhy::new();
        phy.fail_send_ltp = true;

        let err = FrlTrainer::new(scdc, phy)
            .train_at_rate(RATE, &TrainingConfig::default())
            .unwrap_err();
        assert_eq!(err, TrainingError::Phy(()));
    }

    #[test]
    fn send_ltp_correct_pattern_per_ltp_req() {
        // Verify the From<LtpReq> → LtpPattern mapping: each LFSR variant
        // must produce the corresponding raw pattern index (1–4).
        let cases: &[(LtpReq, u8)] = &[
            (LtpReq::Lfsr0, 1),
            (LtpReq::Lfsr1, 2),
            (LtpReq::Lfsr2, 3),
            (LtpReq::Lfsr3, 4),
        ];
        for &(req, expected_raw) in cases {
            let mut scdc = SimScdc::new();
            scdc.push(flt_ready());
            scdc.push(frl_started());
            scdc.push(ltp(req)); // sink requests this pattern
            scdc.push(frl_started()); // ltp_req = None → success

            let mut trainer = FrlTrainer::new(scdc, MockPhy::new());
            trainer
                .train_at_rate(RATE, &TrainingConfig::default())
                .unwrap();
            let (_, phy) = trainer.into_parts();

            assert_eq!(
                phy.last_ltp.unwrap().value(),
                expected_raw,
                "LtpReq variant produced wrong LtpPattern raw value"
            );
        }
    }

    // -------------------------------------------------------------------------
    // into_parts
    // -------------------------------------------------------------------------

    #[test]
    fn into_parts_recovers_scdc_and_phy() {
        let mut scdc = SimScdc::new();
        scdc.push(flt_ready());
        scdc.push(frl_started());
        scdc.push(frl_started()); // phase 4: ltp_req = None

        let mut trainer = FrlTrainer::new(scdc, MockPhy::new());
        trainer
            .train_at_rate(RATE, &TrainingConfig::default())
            .unwrap();

        let (scdc, phy) = trainer.into_parts();
        assert_eq!(scdc.written_config.unwrap().rate, RATE);
        assert_eq!(phy.frl_rate, Some(RATE));
    }

    // -------------------------------------------------------------------------
    // Diagnostics: TrainingTrace event sequences
    // -------------------------------------------------------------------------

    #[cfg(feature = "alloc")]
    mod trace_tests {
        extern crate std;
        use std::vec::Vec;

        use crate::trace::TrainingEvent;
        use crate::types::{FfeLevels, LtpReq};

        use super::{
            FrlTrainer, MockPhy, RATE, SimScdc, TrainingConfig, TrainingOutcome, cfg, flt_ready,
            frl_started, ltp, not_ready,
        };

        #[test]
        fn trace_successful_run() {
            let mut scdc = SimScdc::new();
            // Phase 2: 2 failed reads then flt_ready (after_iterations = 2)
            scdc.push(not_ready());
            scdc.push(not_ready());
            scdc.push(flt_ready());
            // Phase 3: 1 failed read then frl_start (after_iterations = 1)
            scdc.push(flt_ready());
            scdc.push(frl_started());
            // Phase 4: two pattern requests then success (after_iterations = 2)
            scdc.push(ltp(LtpReq::Lfsr0));
            scdc.push(ltp(LtpReq::Lfsr2));
            scdc.push(frl_started()); // ltp_req = None

            let (outcome, trace) = FrlTrainer::new(scdc, MockPhy::new())
                .train_at_rate_traced(RATE, &TrainingConfig::default())
                .unwrap();

            assert_eq!(
                outcome,
                TrainingOutcome::Success {
                    achieved_rate: RATE
                }
            );
            assert_eq!(trace.rate, RATE);
            assert_eq!(
                trace.events,
                [
                    TrainingEvent::RateConfigured {
                        rate: RATE,
                        ffe_levels: FfeLevels::Ffe0
                    },
                    TrainingEvent::FltReadyReceived {
                        after_iterations: 2
                    },
                    TrainingEvent::FrlStartReceived {
                        after_iterations: 1
                    },
                    TrainingEvent::LtpPatternRequested {
                        pattern: LtpReq::Lfsr0
                    },
                    TrainingEvent::LtpPatternRequested {
                        pattern: LtpReq::Lfsr2
                    },
                    TrainingEvent::AllLanesSatisfied {
                        after_iterations: 2
                    },
                ]
            );
        }

        #[test]
        fn trace_phase2_timeout() {
            let mut scdc = SimScdc::new();
            scdc.push(not_ready());
            scdc.push(not_ready());
            scdc.push(not_ready()); // i reaches flt_ready_timeout = 3

            let (_outcome, trace) = FrlTrainer::new(scdc, MockPhy::new())
                .train_at_rate_traced(RATE, &cfg(3, 10, 10))
                .unwrap();

            assert_eq!(
                trace.events,
                [
                    TrainingEvent::RateConfigured {
                        rate: RATE,
                        ffe_levels: FfeLevels::Ffe0
                    },
                    TrainingEvent::FltReadyTimeout {
                        iterations_elapsed: 3
                    },
                ]
            );
        }

        #[test]
        fn trace_phase3_timeout() {
            let mut scdc = SimScdc::new();
            scdc.push(flt_ready()); // phase 2: immediate (after_iterations = 0)
            scdc.push(flt_ready()); // frl_start = false
            scdc.push(flt_ready());
            scdc.push(flt_ready()); // i reaches frl_start_timeout = 3

            let (_outcome, trace) = FrlTrainer::new(scdc, MockPhy::new())
                .train_at_rate_traced(RATE, &cfg(10, 3, 10))
                .unwrap();

            assert_eq!(
                trace.events,
                [
                    TrainingEvent::RateConfigured {
                        rate: RATE,
                        ffe_levels: FfeLevels::Ffe0
                    },
                    TrainingEvent::FltReadyReceived {
                        after_iterations: 0
                    },
                    TrainingEvent::FrlStartTimeout {
                        iterations_elapsed: 3
                    },
                ]
            );
        }

        #[test]
        fn trace_phase4_timeout() {
            let mut scdc = SimScdc::new();
            scdc.push(flt_ready());
            scdc.push(frl_started());
            scdc.push(ltp(LtpReq::Lfsr1)); // i = 1, new pattern
            scdc.push(ltp(LtpReq::Lfsr3)); // i = 2, new pattern
            scdc.push(ltp(LtpReq::Lfsr3)); // i = 3, same — no new event → timeout

            let (_outcome, trace) = FrlTrainer::new(scdc, MockPhy::new())
                .train_at_rate_traced(RATE, &cfg(10, 10, 3))
                .unwrap();

            assert_eq!(
                trace.events,
                [
                    TrainingEvent::RateConfigured {
                        rate: RATE,
                        ffe_levels: FfeLevels::Ffe0
                    },
                    TrainingEvent::FltReadyReceived {
                        after_iterations: 0
                    },
                    TrainingEvent::FrlStartReceived {
                        after_iterations: 0
                    },
                    TrainingEvent::LtpPatternRequested {
                        pattern: LtpReq::Lfsr1
                    },
                    TrainingEvent::LtpPatternRequested {
                        pattern: LtpReq::Lfsr3
                    },
                    TrainingEvent::LtpLoopTimeout {
                        iterations_elapsed: 3
                    },
                ]
            );
        }

        #[test]
        fn trace_ltp_pattern_on_transition_only() {
            // A sink that holds Lfsr1 for 3 iterations then changes to Lfsr2 produces
            // exactly one LtpPatternRequested{Lfsr1}, not three.
            let mut scdc = SimScdc::new();
            scdc.push(flt_ready());
            scdc.push(frl_started());
            scdc.push(ltp(LtpReq::Lfsr1));
            scdc.push(ltp(LtpReq::Lfsr1));
            scdc.push(ltp(LtpReq::Lfsr1));
            scdc.push(ltp(LtpReq::Lfsr2));
            scdc.push(frl_started()); // ltp_req = None → success (after_iterations = 4)

            let (_outcome, trace) = FrlTrainer::new(scdc, MockPhy::new())
                .train_at_rate_traced(RATE, &TrainingConfig::default())
                .unwrap();

            let ltp_events: Vec<_> = trace
                .events
                .iter()
                .filter(|e| matches!(e, TrainingEvent::LtpPatternRequested { .. }))
                .collect();
            assert_eq!(ltp_events.len(), 2); // Lfsr1 once, Lfsr2 once
            assert_eq!(
                ltp_events[0],
                &TrainingEvent::LtpPatternRequested {
                    pattern: LtpReq::Lfsr1
                }
            );
            assert_eq!(
                ltp_events[1],
                &TrainingEvent::LtpPatternRequested {
                    pattern: LtpReq::Lfsr2
                }
            );

            // AllLanesSatisfied should reflect 4 loop iterations (3 × Lfsr1 + 1 × Lfsr2)
            assert!(trace.events.contains(&TrainingEvent::AllLanesSatisfied {
                after_iterations: 4
            }));
        }

        #[test]
        fn trace_config_recorded_and_timeout_interpretable() {
            let config = cfg(5, 7, 9);
            let mut scdc = SimScdc::new();
            // Drive to a phase 2 timeout so we can read iterations_elapsed against config.
            for _ in 0..5 {
                scdc.push(not_ready());
            }

            let (_outcome, trace) = FrlTrainer::new(scdc, MockPhy::new())
                .train_at_rate_traced(RATE, &config)
                .unwrap();

            assert_eq!(trace.config, config);
            // The timeout event count should equal the configured limit.
            assert!(trace.events.contains(&TrainingEvent::FltReadyTimeout {
                iterations_elapsed: 5
            }));
            assert_eq!(trace.config.flt_ready_timeout, 5);
        }
    }
}
