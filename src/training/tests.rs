extern crate std;
use std::vec::Vec;

use super::sim::{PhyCall, PhyOp, SimPhy, SimSink, SinkCall, SinkOp, all};
use super::*;
use crate::lts::{FLT_UPDATE, FRL_START, Lanes, SOURCE_TEST_UPDATE, uniform};
use crate::types::LtpReq;
use crate::types::SourceTestConfig;
use crate::warning::{Trained, TrainingWarning};
use hdmi_hal::phy::LtpPattern;

const RATE: HdmiForumFrl = HdmiForumFrl::Rate6Gbps4Lanes;

const NO_FLAGS: UpdateFlags = UpdateFlags {
    source_test_update: false,
    frl_start: false,
    flt_update: false,
};

const NO_TIMEOUT: SourceTestConfig = SourceTestConfig {
    flt_no_timeout: true,
};

fn run(
    sink: SimSink,
    rates: &[HdmiForumFrl],
    config: &TrainingConfig,
) -> (TrainingOutcome, SimSink, SimPhy) {
    let mut trainer = FrlTrainer::new(sink, SimPhy::new());
    let outcome = trainer.train(rates, config).unwrap().outcome;
    let (sink, phy) = trainer.into_parts();
    (outcome, sink, phy)
}

fn fallback(reason: FallbackReason) -> TrainingOutcome {
    TrainingOutcome::FallbackRequired { reason }
}

fn requests(lane0: LtpReq, lane1: LtpReq, lane2: LtpReq, lane3: LtpReq) -> LtpRequests {
    LtpRequests {
        lane0,
        lane1,
        lane2,
        lane3,
    }
}

fn patterns(
    lane0: Option<LtpPattern>,
    lane1: Option<LtpPattern>,
    lane2: Option<LtpPattern>,
    lane3: Option<LtpPattern>,
) -> LanePatterns {
    LanePatterns {
        lane0,
        lane1,
        lane2,
        lane3,
    }
}

/// The patterns the PHY was sent during LTS:3, after LTS:2's Nyquist clock and stop.
fn ltp_sent(phy: &SimPhy) -> Vec<LanePatterns> {
    phy.calls
        .iter()
        .filter_map(|call| match call {
            PhyCall::SendLtp(p) => Some(*p),
            _ => None,
        })
        .skip(2)
        .collect()
}

/// The TxFFE levels of each `adjust_equalization` call after LTS:2's reset.
fn levels_sent(phy: &SimPhy) -> Vec<[u8; 4]> {
    phy.calls
        .iter()
        .filter_map(|call| match call {
            PhyCall::AdjustEqualization(eq) => Some([
                eq.lane0.tx_ffe_level.value(),
                eq.lane1.tx_ffe_level.value(),
                eq.lane2.tx_ffe_level.value(),
                eq.lane3.map_or(0, |l| l.tx_ffe_level.value()),
            ]),
            _ => None,
        })
        .skip(1)
        .collect()
}

fn count(sink: &SimSink, call: fn(&SinkCall) -> bool) -> usize {
    sink.calls.iter().filter(|c| call(c)).count()
}

// --- TrainingConfig

#[test]
fn training_config_defaults() {
    let config = TrainingConfig::default();
    assert_eq!(config.ffe_levels, FfeLevels::new(3).unwrap());
    assert_eq!(config.flt_ready_polls, 50);
    assert_eq!(config.ltp_polls, 100);
    assert_eq!(config.frl_start_polls, 100);
    assert_eq!(config.no_timeout_poll_cap, 500);
    assert_eq!(config.max_retrains, 3);
    assert!(config.exit_to_tmds_on_error);
}

// --- TrainingError

#[test]
fn training_error_variants_are_distinct() {
    let exit = TmdsExit::Exited;
    let s: TrainingError<u8, u8> = TrainingError::Scdc { error: 1, exit };
    let p: TrainingError<u8, u8> = TrainingError::Phy { error: 1, exit };
    let x: TrainingError<u8, u8> = TrainingError::ExitFailed {
        reason: FallbackReason::TrainingTimeout,
        error: ExitError {
            scdc: Some(1),
            phy: None,
        },
    };
    assert_ne!(s, p);
    assert_ne!(s, x);
    assert_ne!(p, x);
}

// --- The main path: LTS:2 → LTS:3 → LTS:P

#[test]
fn trains_through_lts_2_3_and_p() {
    let sink = SimSink::new()
        .flt_ready_after(1)
        .round(0, all(LtpReq::Lfsr0))
        .round(2, all(LtpReq::None))
        .frl_start_after(1);
    let (outcome, sink, phy) = run(sink, &[RATE], &TrainingConfig::default());

    assert_eq!(
        outcome,
        TrainingOutcome::Success {
            achieved_rate: RATE
        }
    );
    let flt_update = FLT_UPDATE;
    let frl_start = FRL_START;
    let lfsr0 = Some(LtpPattern::Lfsr0);
    assert_eq!(
        sink.calls,
        [
            // LTS:2
            SinkCall::ReadUpdateFlags(NO_FLAGS),
            SinkCall::ReadSourceTestConfig(SourceTestConfig::default()),
            SinkCall::ReadFltReady(false),
            SinkCall::ReadFltReady(true),
            SinkCall::ClearUpdateFlags(flt_update),
            SinkCall::WriteConfig0Defaults,
            SinkCall::WriteFrlConfig(FrlConfig {
                rate: RATE,
                ffe_levels: FfeLevels::new(3).unwrap(),
            }),
            // LTS:3
            SinkCall::ReadUpdateFlags(flt_update),
            SinkCall::ReadLtpRequests(all(LtpReq::Lfsr0)),
            SinkCall::ClearUpdateFlags(flt_update),
            SinkCall::ReadUpdateFlags(NO_FLAGS),
            SinkCall::ReadUpdateFlags(NO_FLAGS),
            SinkCall::ReadUpdateFlags(flt_update),
            SinkCall::ReadLtpRequests(all(LtpReq::None)),
            // LTS:P
            SinkCall::ClearUpdateFlags(flt_update),
            SinkCall::ReadUpdateFlags(NO_FLAGS),
            SinkCall::ReadUpdateFlags(frl_start),
            SinkCall::ClearUpdateFlags(frl_start),
        ]
    );
    assert_eq!(
        phy.calls,
        [
            // LTS:2
            PhyCall::AdjustEqualization(Lanes::new(RATE).eq_params()),
            PhyCall::SetFrlRate(RATE),
            PhyCall::SendLtp(uniform(4, Some(LtpPattern::NyquistClock))),
            PhyCall::SendLtp(LanePatterns::default()),
            PhyCall::SetFrlOutput(FrlOutput::GapOnly),
            // LTS:3
            PhyCall::SendLtp(patterns(lfsr0, lfsr0, lfsr0, lfsr0)),
            // LTS:P
            PhyCall::SendLtp(LanePatterns::default()),
            PhyCall::SetFrlOutput(FrlOutput::GapOnly),
        ]
    );
}

#[test]
fn train_at_rate_is_train_with_one_rate() {
    let sink = || {
        SimSink::new()
            .flt_ready_after(0)
            .round(0, all(LtpReq::None))
            .frl_start_after(0)
    };
    let mut trainer = FrlTrainer::new(sink(), SimPhy::new());
    let outcome = trainer
        .train_at_rate(RATE, &TrainingConfig::default())
        .unwrap()
        .outcome;
    let (at_rate, _) = trainer.into_parts();
    let (expected, list, _) = run(sink(), &[RATE], &TrainingConfig::default());
    assert_eq!(outcome, expected);
    assert_eq!(at_rate.calls, list.calls);
}

#[test]
fn a_rate_list_the_procedure_cannot_run_is_rejected_without_io() {
    use HdmiForumFrl::*;
    let cases: [(&[HdmiForumFrl], usize, HdmiForumFrl); 5] = [
        (&[NotSupported], 0, NotSupported),
        (&[Rate12Gbps4Lanes, NotSupported], 1, NotSupported),
        (&[Rate12Gbps4Lanes, Rate12Gbps4Lanes], 1, Rate12Gbps4Lanes),
        (&[Rate10Gbps4Lanes, Rate12Gbps4Lanes], 1, Rate12Gbps4Lanes),
        (
            &[Rate12Gbps4Lanes, Rate6Gbps3Lanes, Rate6Gbps4Lanes],
            2,
            Rate6Gbps4Lanes,
        ),
    ];
    for (rates, index, rate) in cases {
        let mut trainer = FrlTrainer::new(SimSink::new().flt_ready_after(0), SimPhy::new());
        let result = trainer.train(rates, &TrainingConfig::default());
        assert_eq!(
            result,
            Err(TrainingError::InvalidRates { index, rate }),
            "{rates:?}"
        );
        let (sink, phy) = trainer.into_parts();
        assert!(sink.calls.is_empty(), "{rates:?}");
        assert!(phy.calls.is_empty(), "{rates:?}");
    }
}

#[test]
fn every_rate_in_descending_order_is_accepted() {
    use HdmiForumFrl::*;
    let rates = [
        Rate12Gbps4Lanes,
        Rate10Gbps4Lanes,
        Rate8Gbps4Lanes,
        Rate6Gbps4Lanes,
        Rate6Gbps3Lanes,
        Rate3Gbps3Lanes,
    ];
    // The sink asks for a lower rate at every step: training walks the whole list.
    let mut sink = SimSink::new().flt_ready_after(0);
    for _ in 0..rates.len() {
        sink = sink.round(0, all(LtpReq::RateChange));
    }
    let (outcome, sink, _) = run(sink, &rates, &TrainingConfig::default());
    assert_eq!(outcome, fallback(FallbackReason::RatesExhausted));
    let mut configured = rates.to_vec();
    configured.push(NotSupported);
    assert_eq!(frl_configs(&sink), configured);
}

#[test]
fn an_empty_rate_list_is_rejected_without_io() {
    let mut trainer = FrlTrainer::new(SimSink::new(), SimPhy::new());
    let result = trainer.train(&[], &TrainingConfig::default());
    assert_eq!(result, Err(TrainingError::NoRates));
    let (sink, phy) = trainer.into_parts();
    assert!(sink.calls.is_empty());
    assert!(phy.calls.is_empty());
}

// --- LTS:2

#[test]
fn flt_ready_timeout_after_exactly_the_poll_limit() {
    let config = TrainingConfig {
        flt_ready_polls: 5,
        ..TrainingConfig::default()
    };
    let (outcome, sink, _) = run(SimSink::new(), &[RATE], &config);
    assert_eq!(outcome, fallback(FallbackReason::FltReadyTimeout));
    assert_eq!(count(&sink, |c| matches!(c, SinkCall::ReadFltReady(_))), 5);
}

#[test]
fn flt_ready_on_the_last_allowed_poll_continues() {
    let config = TrainingConfig {
        flt_ready_polls: 5,
        ..TrainingConfig::default()
    };
    let sink = SimSink::new()
        .flt_ready_after(4)
        .round(0, all(LtpReq::None))
        .frl_start_after(0);
    let (outcome, _, _) = run(sink, &[RATE], &config);
    assert_eq!(
        outcome,
        TrainingOutcome::Success {
            achieved_rate: RATE
        }
    );
}

#[test]
fn ffe_levels_written_are_limited_for_the_rate() {
    let config = TrainingConfig {
        ffe_levels: FfeLevels::MAX,
        ..TrainingConfig::default()
    };
    let rate = HdmiForumFrl::Rate12Gbps4Lanes;
    let sink = SimSink::new()
        .flt_ready_after(0)
        .round(0, all(LtpReq::None))
        .frl_start_after(0);
    let (_, sink, _) = run(sink, &[rate], &config);
    assert!(sink.calls.contains(&SinkCall::WriteFrlConfig(FrlConfig {
        rate,
        ffe_levels: FfeLevels::new(3).unwrap(),
    })));
}

// --- LTS:3

#[test]
fn each_lane_gets_its_own_pattern() {
    let sink = SimSink::new()
        .flt_ready_after(0)
        .round(
            0,
            requests(LtpReq::Lfsr0, LtpReq::Lfsr1, LtpReq::Lfsr2, LtpReq::Lfsr3),
        )
        .round(0, all(LtpReq::None))
        .frl_start_after(0);
    let (_, _, phy) = run(sink, &[RATE], &TrainingConfig::default());
    assert_eq!(
        ltp_sent(&phy)[0],
        patterns(
            Some(LtpPattern::Lfsr0),
            Some(LtpPattern::Lfsr1),
            Some(LtpPattern::Lfsr2),
            Some(LtpPattern::Lfsr3),
        )
    );
}

#[test]
fn a_lane_requesting_none_keeps_its_pattern() {
    let sink = SimSink::new()
        .flt_ready_after(0)
        .round(0, all(LtpReq::Lfsr0))
        .round(
            0,
            requests(LtpReq::None, LtpReq::AllOnes, LtpReq::None, LtpReq::None),
        )
        .round(0, all(LtpReq::None))
        .frl_start_after(0);
    let (_, _, phy) = run(sink, &[RATE], &TrainingConfig::default());
    let lfsr0 = Some(LtpPattern::Lfsr0);
    assert_eq!(
        ltp_sent(&phy)[1],
        patterns(lfsr0, Some(LtpPattern::AllOnes), lfsr0, lfsr0)
    );
}

#[test]
fn nyquist_clock_without_flt_no_timeout_keeps_the_previous_pattern() {
    let sink = SimSink::new()
        .flt_ready_after(0)
        .round(0, all(LtpReq::Lfsr2))
        .round(0, all(LtpReq::NyquistClock))
        .round(0, all(LtpReq::None))
        .frl_start_after(0);
    let (_, _, phy) = run(sink, &[RATE], &TrainingConfig::default());
    let sent = ltp_sent(&phy);
    assert_eq!(sent[1], sent[0]);
    assert_eq!(sent[1].lane0, Some(LtpPattern::Lfsr2));
}

#[test]
fn nyquist_clock_with_flt_no_timeout_is_driven() {
    let sink = SimSink::new()
        .source_test(NO_TIMEOUT)
        .flt_ready_after(0)
        .round(0, all(LtpReq::NyquistClock))
        .round(0, all(LtpReq::None))
        .frl_start_after(0);
    let (_, _, phy) = run(sink, &[RATE], &TrainingConfig::default());
    assert_eq!(
        ltp_sent(&phy)[0],
        uniform(4, Some(LtpPattern::NyquistClock))
    );
}

#[test]
fn ffe_change_raises_the_lane_level_and_holds_it_at_the_maximum() {
    let config = TrainingConfig {
        ffe_levels: FfeLevels::new(2).unwrap(),
        ..TrainingConfig::default()
    };
    let raise_lane1 = requests(LtpReq::None, LtpReq::FfeChange, LtpReq::None, LtpReq::None);
    let sink = SimSink::new()
        .flt_ready_after(0)
        .round(0, raise_lane1)
        .round(0, raise_lane1)
        .round(0, raise_lane1)
        .round(0, all(LtpReq::None))
        .frl_start_after(0);
    let (_, _, phy) = run(sink, &[RATE], &config);
    // The third request finds the lane at the maximum: no change, no PHY update.
    assert_eq!(levels_sent(&phy), [[0, 1, 0, 0], [0, 2, 0, 0]]);
    assert_eq!(ltp_sent(&phy).len(), 3 + 1);
}

#[test]
fn three_lane_rates_ignore_lane_3() {
    let rate = HdmiForumFrl::Rate6Gbps3Lanes;
    let sink = SimSink::new()
        .flt_ready_after(0)
        .round(0, all(LtpReq::Lfsr1))
        .round(
            0,
            requests(LtpReq::FfeChange, LtpReq::None, LtpReq::None, LtpReq::Lfsr3),
        )
        .round(
            0,
            requests(LtpReq::None, LtpReq::None, LtpReq::None, LtpReq::Lfsr3),
        )
        .frl_start_after(0);
    let config = TrainingConfig {
        ffe_levels: FfeLevels::new(3).unwrap(),
        ..TrainingConfig::default()
    };
    let (outcome, _, phy) = run(sink, &[rate], &config);
    assert_eq!(
        outcome,
        TrainingOutcome::Success {
            achieved_rate: rate
        }
    );
    let lfsr1 = Some(LtpPattern::Lfsr1);
    assert_eq!(ltp_sent(&phy)[0], patterns(lfsr1, lfsr1, lfsr1, None));
    let eq = phy.calls.iter().rev().find_map(|call| match call {
        PhyCall::AdjustEqualization(eq) => Some(*eq),
        _ => None,
    });
    assert_eq!(eq.map(|eq| eq.lane3), Some(None));
}

#[test]
fn a_reserved_request_keeps_the_lanes_state() {
    let config = TrainingConfig {
        ffe_levels: FfeLevels::new(3).unwrap(),
        ..TrainingConfig::default()
    };
    let sink = SimSink::new()
        .flt_ready_after(0)
        .round(0, all(LtpReq::Lfsr0))
        .round(0, all(LtpReq::FfeChange))
        .round(
            0,
            requests(
                LtpReq::Reserved(0x9),
                LtpReq::AllOnes,
                LtpReq::Reserved(0xD),
                LtpReq::None,
            ),
        )
        .round(0, all(LtpReq::None))
        .frl_start_after(0);
    let (outcome, _, phy) = run(sink, &[RATE], &config);
    assert_eq!(
        outcome,
        TrainingOutcome::Success {
            achieved_rate: RATE
        }
    );
    let lfsr0 = Some(LtpPattern::Lfsr0);
    assert_eq!(
        ltp_sent(&phy)[2],
        patterns(lfsr0, Some(LtpPattern::AllOnes), lfsr0, lfsr0)
    );
    // The reserved values changed no level: the one raise stands.
    assert_eq!(levels_sent(&phy), [[1, 1, 1, 1]]);
}

#[test]
fn a_reserved_request_is_neither_a_pass_nor_a_rate_change() {
    let config = TrainingConfig {
        ltp_polls: 3,
        ..TrainingConfig::default()
    };
    for other in [LtpReq::None, LtpReq::RateChange] {
        let sink = SimSink::new()
            .flt_ready_after(0)
            .round(0, requests(other, other, other, LtpReq::Reserved(0xA)));
        let (outcome, _, _) = run(sink, &[RATE, HdmiForumFrl::Rate3Gbps3Lanes], &config);
        assert_eq!(
            outcome,
            fallback(FallbackReason::TrainingTimeout),
            "{other:?}"
        );
    }
}

#[test]
fn three_lane_rates_ignore_a_reserved_lane_3() {
    let rate = HdmiForumFrl::Rate6Gbps3Lanes;
    let sink = SimSink::new()
        .flt_ready_after(0)
        .round(
            0,
            requests(
                LtpReq::None,
                LtpReq::None,
                LtpReq::None,
                LtpReq::Reserved(0x9),
            ),
        )
        .frl_start_after(0);
    let (outcome, _, _) = run(sink, &[rate], &TrainingConfig::default());
    assert_eq!(
        outcome,
        TrainingOutcome::Success {
            achieved_rate: rate
        }
    );
}

#[test]
fn undefined_requests_are_returned_as_warnings() {
    let rate = HdmiForumFrl::Rate6Gbps3Lanes;
    let lane1 = requests(
        LtpReq::Lfsr0,
        LtpReq::Reserved(0x9),
        LtpReq::Lfsr0,
        LtpReq::None,
    );
    let sink = SimSink::new()
        .flt_ready_after(0)
        .round(0, lane1)
        .round(
            0,
            requests(
                LtpReq::Lfsr0,
                LtpReq::Reserved(0xB),
                LtpReq::Lfsr0,
                LtpReq::Reserved(0xD),
            ),
        )
        .round(0, all(LtpReq::None))
        .frl_start_after(0);
    let trained = FrlTrainer::new(sink, SimPhy::new())
        .train(&[rate], &TrainingConfig::default())
        .unwrap();
    assert_eq!(
        trained.outcome,
        TrainingOutcome::Success {
            achieved_rate: rate
        }
    );
    let warnings = [
        TrainingWarning::UndefinedLtpRequest {
            lane: 1,
            value: 0xB,
            count: 2,
            in_use: true,
        },
        TrainingWarning::UndefinedLtpRequest {
            lane: 3,
            value: 0xD,
            count: 1,
            in_use: false,
        },
    ];
    assert!(trained.iter_warnings().eq(warnings.iter()));
}

#[test]
fn an_undefined_value_on_the_passing_round_is_reported() {
    // The round that passes the three lanes in use still has lane 3's value looked at.
    let rate = HdmiForumFrl::Rate6Gbps3Lanes;
    let sink = SimSink::new()
        .flt_ready_after(0)
        .round(
            0,
            requests(
                LtpReq::None,
                LtpReq::None,
                LtpReq::None,
                LtpReq::Reserved(0xD),
            ),
        )
        .frl_start_after(0);
    let trained = FrlTrainer::new(sink, SimPhy::new())
        .train(&[rate], &TrainingConfig::default())
        .unwrap();
    let warning = TrainingWarning::UndefinedLtpRequest {
        lane: 3,
        value: 0xD,
        count: 1,
        in_use: false,
    };
    assert!(trained.iter_warnings().eq([warning].iter()));
}

#[test]
fn a_fallback_carries_its_warnings() {
    let config = TrainingConfig {
        ltp_polls: 2,
        ..TrainingConfig::default()
    };
    let sink = SimSink::new()
        .flt_ready_after(0)
        .round(0, all(LtpReq::Reserved(0xA)));
    let trained = FrlTrainer::new(sink, SimPhy::new())
        .train(&[RATE], &config)
        .unwrap();
    assert_eq!(trained.outcome, fallback(FallbackReason::TrainingTimeout));
    assert_eq!(trained.iter_warnings().count(), 4);
}

#[test]
fn a_clean_attempt_has_no_warnings() {
    let sink = SimSink::new()
        .flt_ready_after(0)
        .round(0, all(LtpReq::Lfsr0))
        .round(0, all(LtpReq::None))
        .frl_start_after(0);
    let trained = FrlTrainer::new(sink, SimPhy::new())
        .train(&[RATE], &TrainingConfig::default())
        .unwrap();
    assert_eq!(trained.iter_warnings().count(), 0);
}

#[test]
fn a_rate_change_on_some_lanes_keeps_their_state() {
    let sink = SimSink::new()
        .flt_ready_after(0)
        .round(0, all(LtpReq::Lfsr0))
        .round(
            0,
            requests(
                LtpReq::RateChange,
                LtpReq::Lfsr1,
                LtpReq::None,
                LtpReq::None,
            ),
        )
        .round(0, all(LtpReq::None))
        .frl_start_after(0);
    let (outcome, _, phy) = run(sink, &[RATE], &TrainingConfig::default());
    assert_eq!(
        outcome,
        TrainingOutcome::Success {
            achieved_rate: RATE
        }
    );
    let lfsr0 = Some(LtpPattern::Lfsr0);
    assert_eq!(
        ltp_sent(&phy)[1],
        patterns(lfsr0, Some(LtpPattern::Lfsr1), lfsr0, lfsr0)
    );
}

#[test]
fn training_timeout_after_exactly_the_poll_limit() {
    let config = TrainingConfig {
        ltp_polls: 7,
        ..TrainingConfig::default()
    };
    let sink = SimSink::new().flt_ready_after(0);
    let (outcome, sink, _) = run(sink, &[RATE], &config);
    assert_eq!(outcome, fallback(FallbackReason::TrainingTimeout));
    // One read in LTS:2, exactly the limit in LTS:3, then LTS:L's read.
    assert_eq!(
        count(&sink, |c| matches!(c, SinkCall::ReadUpdateFlags(_))),
        1 + 7 + 1
    );
}

#[test]
fn polls_that_see_an_update_count_towards_the_limit() {
    let config = TrainingConfig {
        ltp_polls: 3,
        ..TrainingConfig::default()
    };
    let sink = SimSink::new()
        .flt_ready_after(0)
        .round(0, all(LtpReq::Lfsr0))
        .round(0, all(LtpReq::Lfsr1))
        .round(0, all(LtpReq::Lfsr2))
        .round(0, all(LtpReq::None));
    let (outcome, _, _) = run(sink, &[RATE], &config);
    assert_eq!(outcome, fallback(FallbackReason::TrainingTimeout));
}

#[test]
fn all_rate_change_on_the_last_rate_exhausts_the_list() {
    let sink = SimSink::new()
        .flt_ready_after(0)
        .round(0, all(LtpReq::RateChange));
    let (outcome, _, _) = run(sink, &[RATE], &TrainingConfig::default());
    assert_eq!(outcome, fallback(FallbackReason::RatesExhausted));
}

// --- LTS:4

const R12: HdmiForumFrl = HdmiForumFrl::Rate12Gbps4Lanes;
const R10: HdmiForumFrl = HdmiForumFrl::Rate10Gbps4Lanes;

fn frl_configs(sink: &SimSink) -> Vec<HdmiForumFrl> {
    sink.calls
        .iter()
        .filter_map(|call| match call {
            SinkCall::WriteFrlConfig(config) => Some(config.rate),
            _ => None,
        })
        .collect()
}

fn phy_rates(phy: &SimPhy) -> Vec<HdmiForumFrl> {
    phy.calls
        .iter()
        .filter_map(|call| match call {
            PhyCall::SetFrlRate(rate) => Some(*rate),
            _ => None,
        })
        .collect()
}

#[test]
fn a_rate_change_steps_down_to_the_next_rate() {
    let sink = SimSink::new()
        .flt_ready_after(0)
        .round(0, all(LtpReq::RateChange))
        .round(0, all(LtpReq::Lfsr0))
        .round(0, all(LtpReq::None))
        .frl_start_after(0);
    let (outcome, sink, phy) = run(sink, &[R12, R10], &TrainingConfig::default());
    assert_eq!(outcome, TrainingOutcome::Success { achieved_rate: R10 });
    assert_eq!(frl_configs(&sink), [R12, R10]);
    assert_eq!(phy_rates(&phy), [R12, R10]);
    // FLT_ready is awaited only in LTS:2.
    assert_eq!(count(&sink, |c| matches!(c, SinkCall::ReadFltReady(_))), 1);
}

#[test]
fn a_rate_change_resets_the_lanes() {
    let config = TrainingConfig {
        ffe_levels: FfeLevels::new(3).unwrap(),
        ..TrainingConfig::default()
    };
    let sink = SimSink::new()
        .flt_ready_after(0)
        .round(0, all(LtpReq::Lfsr0))
        .round(0, all(LtpReq::FfeChange))
        .round(0, all(LtpReq::RateChange))
        .round(
            0,
            requests(LtpReq::Lfsr1, LtpReq::None, LtpReq::None, LtpReq::None),
        )
        .round(0, all(LtpReq::None))
        .frl_start_after(0);
    let (_, _, phy) = run(sink, &[R12, R10], &config);
    // Raised to 1 at 12 Gbps, then reset to 0 for 10 Gbps.
    assert_eq!(levels_sent(&phy), [[1, 1, 1, 1], [0, 0, 0, 0]]);
    let sent = ltp_sent(&phy);
    // LTS:4 stops the patterns; at the new rate only lane 0 has one.
    assert_eq!(sent[2], LanePatterns::default());
    assert_eq!(sent[3], patterns(Some(LtpPattern::Lfsr1), None, None, None));
}

#[test]
fn a_rate_change_can_move_to_three_lanes() {
    let rate = HdmiForumFrl::Rate6Gbps3Lanes;
    let sink = SimSink::new()
        .flt_ready_after(0)
        .round(0, all(LtpReq::RateChange))
        .round(0, all(LtpReq::Lfsr2))
        .round(
            0,
            requests(LtpReq::None, LtpReq::None, LtpReq::None, LtpReq::Lfsr3),
        )
        .frl_start_after(0);
    let (outcome, _, phy) = run(sink, &[RATE, rate], &TrainingConfig::default());
    assert_eq!(
        outcome,
        TrainingOutcome::Success {
            achieved_rate: rate
        }
    );
    let lfsr2 = Some(LtpPattern::Lfsr2);
    assert_eq!(ltp_sent(&phy)[1], patterns(lfsr2, lfsr2, lfsr2, None));
}

#[test]
fn each_rate_gets_a_fresh_poll_limit() {
    let config = TrainingConfig {
        ltp_polls: 3,
        ..TrainingConfig::default()
    };
    let sink = SimSink::new()
        .flt_ready_after(0)
        .round(2, all(LtpReq::RateChange))
        .round(2, all(LtpReq::None))
        .frl_start_after(0);
    let (outcome, _, _) = run(sink, &[R12, R10], &config);
    assert_eq!(outcome, TrainingOutcome::Success { achieved_rate: R10 });
}

#[test]
fn rate_changes_past_the_end_of_the_list_exhaust_it() {
    let sink = SimSink::new()
        .flt_ready_after(0)
        .round(0, all(LtpReq::RateChange))
        .round(0, all(LtpReq::RateChange));
    let (outcome, sink, phy) = run(sink, &[R12, R10], &TrainingConfig::default());
    assert_eq!(outcome, fallback(FallbackReason::RatesExhausted));
    assert_eq!(frl_configs(&sink), [R12, R10, HdmiForumFrl::NotSupported]);
    assert_eq!(phy_rates(&phy), [R12, R10, HdmiForumFrl::NotSupported]);
}

// --- LTS:L

#[test]
fn a_fallback_returns_both_ends_to_tmds() {
    let (outcome, sink, phy) = run(SimSink::new(), &[RATE], &TrainingConfig::default());
    assert_eq!(outcome, fallback(FallbackReason::FltReadyTimeout));
    assert_eq!(
        phy.calls,
        [
            PhyCall::SendLtp(LanePatterns::default()),
            PhyCall::SetFrlRate(HdmiForumFrl::NotSupported),
        ]
    );
    assert_eq!(
        sink.calls[sink.calls.len() - 2..],
        [
            SinkCall::WriteFrlConfig(FrlConfig {
                rate: HdmiForumFrl::NotSupported,
                ffe_levels: FfeLevels::default(),
            }),
            // FLT_update is not set, so it is not cleared.
            SinkCall::ReadUpdateFlags(NO_FLAGS),
        ]
    );
}

#[test]
fn a_fallback_clears_a_pending_flt_update() {
    // The final RateChange is not serviced in LTS:3, so FLT_update is still set.
    let sink = SimSink::new()
        .flt_ready_after(0)
        .round(0, all(LtpReq::RateChange));
    let (outcome, sink, _) = run(sink, &[RATE], &TrainingConfig::default());
    assert_eq!(outcome, fallback(FallbackReason::RatesExhausted));
    assert_eq!(
        sink.calls.last(),
        Some(&SinkCall::ClearUpdateFlags(FLT_UPDATE))
    );
}

#[test]
fn every_timeout_ends_in_tmds() {
    let config = TrainingConfig {
        ltp_polls: 2,
        frl_start_polls: 2,
        ..TrainingConfig::default()
    };
    let flt_ready = SimSink::new();
    let training = SimSink::new().flt_ready_after(0);
    let frl_start = SimSink::new()
        .flt_ready_after(0)
        .round(0, all(LtpReq::None));
    for sink in [flt_ready, training, frl_start] {
        let (_, sink, phy) = run(sink, &[RATE], &config);
        assert_eq!(frl_configs(&sink).last(), Some(&HdmiForumFrl::NotSupported));
        assert_eq!(phy_rates(&phy).last(), Some(&HdmiForumFrl::NotSupported));
    }
}

/// Runs `sink` and `phy` over [`RATE`] with the default config.
fn run_with(sink: SimSink, phy: SimPhy) -> (Result<TrainingOutcome, Error>, SimSink, SimPhy) {
    let mut trainer = FrlTrainer::new(sink, phy);
    let result = trainer
        .train(&[RATE], &TrainingConfig::default())
        .map(|trained| trained.outcome);
    let (sink, phy) = trainer.into_parts();
    (result, sink, phy)
}

type Error = TrainingError<(), ()>;

/// LTS:L's error when the given ends failed.
fn exit_error(scdc: bool, phy: bool) -> ExitError<(), ()> {
    ExitError {
        scdc: scdc.then_some(()),
        phy: phy.then_some(()),
    }
}

/// The error for a fallback whose LTS:L failed on the given ends.
fn exit_failed(reason: FallbackReason, scdc: bool, phy: bool) -> Result<TrainingOutcome, Error> {
    Err(TrainingError::ExitFailed {
        reason,
        error: exit_error(scdc, phy),
    })
}

#[test]
fn every_lts_l_step_is_attempted_when_one_fails() {
    // `FLT_ready` never asserts, so LTS:L is the only place the PHY is called and
    // `Config_1` is written; its `Update_0` read is the second one.
    let timeout = FallbackReason::FltReadyTimeout;
    let cases = [
        (
            SimSink::new(),
            SimPhy::new().fail(PhyOp::SendLtp),
            exit_failed(timeout, false, true),
        ),
        (
            SimSink::new(),
            SimPhy::new().fail(PhyOp::SetFrlRate),
            exit_failed(timeout, false, true),
        ),
        (
            SimSink::new().fail(SinkOp::WriteFrlConfig),
            SimPhy::new(),
            exit_failed(timeout, true, false),
        ),
        (
            SimSink::new().fail_call(SinkOp::ReadUpdateFlags, 2),
            SimPhy::new(),
            exit_failed(timeout, true, false),
        ),
    ];
    for (sink, phy, expected) in cases {
        let (result, sink, phy) = run_with(sink, phy);
        assert_eq!(result, expected);
        let stopped = phy
            .calls
            .contains(&PhyCall::SendLtp(LanePatterns::default()));
        let phy_in_tmds = phy_rates(&phy) == [HdmiForumFrl::NotSupported];
        let sink_in_tmds = frl_configs(&sink) == [HdmiForumFrl::NotSupported];
        let flags_read = count(&sink, |call| matches!(call, SinkCall::ReadUpdateFlags(_))) == 2;
        // Exactly one step failed; the other three ran.
        let ran = [stopped, phy_in_tmds, sink_in_tmds, flags_read];
        assert_eq!(ran.iter().filter(|ran| !**ran).count(), 1, "{ran:?}");
    }
}

#[test]
fn a_failed_flt_update_clear_in_lts_l_is_returned() {
    // The final RateChange is left pending, so LTS:L clears it: the last clear.
    let sink = || {
        SimSink::new()
            .flt_ready_after(0)
            .round(0, all(LtpReq::RateChange))
    };
    let (_, reference, _) = run_with(sink(), SimPhy::new());
    let clears = count(&reference, |call| {
        matches!(call, SinkCall::ClearUpdateFlags(_))
    });
    let (result, sink, phy) = run_with(
        sink().fail_call(SinkOp::ClearUpdateFlags, clears as u32),
        SimPhy::new(),
    );
    assert_eq!(
        result,
        exit_failed(FallbackReason::RatesExhausted, true, false)
    );
    assert_eq!(frl_configs(&sink).last(), Some(&HdmiForumFrl::NotSupported));
    assert_eq!(phy_rates(&phy).last(), Some(&HdmiForumFrl::NotSupported));
}

#[test]
fn lts_l_reports_each_ends_error() {
    let (result, sink, _) = run_with(
        SimSink::new().fail(SinkOp::WriteFrlConfig),
        SimPhy::new().fail(PhyOp::SendLtp),
    );
    assert_eq!(
        result,
        exit_failed(FallbackReason::FltReadyTimeout, true, true)
    );
    // The PHY failing first did not stop the sink's `Update_0` read.
    assert_eq!(
        count(&sink, |call| matches!(call, SinkCall::ReadUpdateFlags(_))),
        2
    );
}

// --- FLT_no_timeout

#[test]
fn flt_no_timeout_suspends_the_flt_ready_limit() {
    let config = TrainingConfig {
        flt_ready_polls: 2,
        ..TrainingConfig::default()
    };
    let sink = SimSink::new()
        .source_test(NO_TIMEOUT)
        .flt_ready_after(10)
        .round(0, all(LtpReq::None))
        .frl_start_after(0);
    let (outcome, sink, _) = run(sink, &[RATE], &config);
    assert_eq!(
        outcome,
        TrainingOutcome::Success {
            achieved_rate: RATE
        }
    );
    assert!(
        sink.calls
            .contains(&SinkCall::ClearUpdateFlags(SOURCE_TEST_UPDATE))
    );
}

#[test]
fn flt_no_timeout_holds_for_every_attempt_while_it_is_set() {
    // A tester sets FLT_no_timeout once and leaves it: Source_Test_Update is cleared by
    // the first attempt, but the second must still see the setting.
    let config = TrainingConfig {
        flt_ready_polls: 2,
        no_timeout_poll_cap: 5,
        ..TrainingConfig::default()
    };
    let mut trainer = FrlTrainer::new(SimSink::new().source_test(NO_TIMEOUT), SimPhy::new());
    for _ in 0..2 {
        let trained = trainer.train(&[RATE], &config).unwrap();
        assert_eq!(
            trained.outcome,
            TrainingOutcome::NoTimeoutHold { rate: RATE }
        );
    }
    let (sink, _) = trainer.into_parts();
    let polls = count(&sink, |c| matches!(c, SinkCall::ReadFltReady(_)));
    assert_eq!(
        polls,
        2 * 5,
        "both attempts poll up to the cap, not the normal limit"
    );
    let clears = count(&sink, |c| {
        *c == SinkCall::ClearUpdateFlags(SOURCE_TEST_UPDATE)
    });
    assert_eq!(clears, 1, "the flag is cleared only while it is set");
}

#[test]
fn the_no_timeout_cap_holds_the_link_in_every_state() {
    let config = TrainingConfig {
        no_timeout_poll_cap: 3,
        ..TrainingConfig::default()
    };
    let lts_2 = SimSink::new().source_test(NO_TIMEOUT);
    let lts_3 = SimSink::new().source_test(NO_TIMEOUT).flt_ready_after(0);
    let lts_p = SimSink::new()
        .source_test(NO_TIMEOUT)
        .flt_ready_after(0)
        .round(0, all(LtpReq::None));
    for (state, sink, configured) in [
        ("LTS:2", lts_2, false),
        ("LTS:3", lts_3, true),
        ("LTS:P", lts_p, true),
    ] {
        let (outcome, sink, phy) = run(sink, &[RATE], &config);
        assert_eq!(
            outcome,
            TrainingOutcome::NoTimeoutHold { rate: RATE },
            "{state}"
        );
        // No LTS:L: nothing is taken down. Past LTS:2, both ends stay at the rate.
        let expected: &[HdmiForumFrl] = if configured { &[RATE] } else { &[] };
        assert_eq!(phy_rates(&phy), expected, "{state}");
        assert_eq!(frl_configs(&sink), expected, "{state}");
    }
}

#[test]
fn the_no_timeout_cap_is_the_limit_in_lts_p() {
    let config = TrainingConfig {
        frl_start_polls: 2,
        no_timeout_poll_cap: 7,
        ..TrainingConfig::default()
    };
    let sink = SimSink::new()
        .source_test(NO_TIMEOUT)
        .flt_ready_after(0)
        .round(0, all(LtpReq::None))
        .frl_start_after(5);
    let (outcome, _, _) = run(sink, &[RATE], &config);
    // FRL_start after 5 polls: past frl_start_polls, within the cap.
    assert_eq!(
        outcome,
        TrainingOutcome::Success {
            achieved_rate: RATE
        }
    );
}

#[test]
fn without_flt_no_timeout_lts_p_still_falls_back() {
    let config = TrainingConfig {
        frl_start_polls: 2,
        ..TrainingConfig::default()
    };
    let sink = SimSink::new()
        .flt_ready_after(0)
        .round(0, all(LtpReq::None))
        .frl_start_after(5);
    let (outcome, _, _) = run(sink, &[RATE], &config);
    assert_eq!(outcome, fallback(FallbackReason::FrlStartTimeout));
}

#[test]
fn flt_no_timeout_set_during_lts_3_suspends_its_limit() {
    let config = TrainingConfig {
        ltp_polls: 2,
        ..TrainingConfig::default()
    };
    let sink = SimSink::new()
        .flt_ready_after(0)
        .round_with_source_test(0, all(LtpReq::Lfsr0), NO_TIMEOUT)
        .round(5, all(LtpReq::None))
        .frl_start_after(0);
    let (outcome, sink, _) = run(sink, &[RATE], &config);
    assert_eq!(
        outcome,
        TrainingOutcome::Success {
            achieved_rate: RATE
        }
    );
    // Once at the start of LTS:2, once when the flag is raised in LTS:3.
    assert_eq!(
        count(&sink, |c| matches!(c, SinkCall::ReadSourceTestConfig(_))),
        2
    );
}

// --- LTS:P

#[test]
fn frl_start_timeout_after_exactly_the_poll_limit() {
    let config = TrainingConfig {
        frl_start_polls: 4,
        ..TrainingConfig::default()
    };
    let sink = SimSink::new()
        .flt_ready_after(0)
        .round(0, all(LtpReq::None));
    let (outcome, sink, _) = run(sink, &[RATE], &config);
    assert_eq!(outcome, fallback(FallbackReason::FrlStartTimeout));
    // One read in LTS:2, one in LTS:3, exactly the limit in LTS:P, then LTS:L's read.
    assert_eq!(
        count(&sink, |c| matches!(c, SinkCall::ReadUpdateFlags(_))),
        1 + 1 + 4 + 1
    );
}

#[test]
fn flt_update_in_lts_p_returns_to_lts_3() {
    let sink = SimSink::new()
        .flt_ready_after(0)
        .round(0, all(LtpReq::Lfsr0))
        .round(0, all(LtpReq::None))
        .round(1, all(LtpReq::Lfsr3))
        .round(0, all(LtpReq::None))
        .frl_start_after(0);
    let (outcome, _, phy) = run(sink, &[RATE], &TrainingConfig::default());
    assert_eq!(
        outcome,
        TrainingOutcome::Success {
            achieved_rate: RATE
        }
    );
    let lfsr3 = Some(LtpPattern::Lfsr3);
    assert_eq!(
        ltp_sent(&phy),
        [
            uniform(4, Some(LtpPattern::Lfsr0)),
            LanePatterns::default(),
            patterns(lfsr3, lfsr3, lfsr3, lfsr3),
            LanePatterns::default(),
        ]
    );
}

/// A sink that passes training and then asks to retrain `retrains` times.
#[test]
fn a_retrain_starts_lts_3_from_no_pattern_and_keeps_the_levels() {
    let config = TrainingConfig {
        ffe_levels: FfeLevels::new(3).unwrap(),
        ..TrainingConfig::default()
    };
    let sink = SimSink::new()
        .flt_ready_after(0)
        .round(0, all(LtpReq::Lfsr0))
        .round(
            0,
            requests(LtpReq::None, LtpReq::FfeChange, LtpReq::None, LtpReq::None),
        )
        .round(0, all(LtpReq::None))
        // The retrain: only lane 1 asks for a pattern.
        .round(
            1,
            requests(LtpReq::None, LtpReq::Lfsr1, LtpReq::None, LtpReq::None),
        )
        .round(0, all(LtpReq::None))
        .frl_start_after(0);
    let (outcome, _, phy) = run(sink, &[RATE], &config);
    assert_eq!(
        outcome,
        TrainingOutcome::Success {
            achieved_rate: RATE
        }
    );
    let lfsr0 = Some(LtpPattern::Lfsr0);
    assert_eq!(
        ltp_sent(&phy),
        [
            uniform(4, lfsr0),
            uniform(4, lfsr0),
            // LTS:P stops every pattern...
            LanePatterns::default(),
            // ...so after the retrain, lanes asking for 0x0 stay without one.
            patterns(None, Some(LtpPattern::Lfsr1), None, None),
            LanePatterns::default(),
        ]
    );
    // Lane 1's raised level carries over the retrain: no reset is sent.
    assert_eq!(levels_sent(&phy), [[0, 1, 0, 0]]);
}

/// A sink that passes LTS:3, then sets FRL_start and FLT_update together in LTS:P (with
/// a request for `Lfsr1`), then passes again and starts.
fn both_flags_sink() -> SimSink {
    SimSink::new()
        .flt_ready_after(0)
        .round(0, all(LtpReq::Lfsr0))
        .round(0, all(LtpReq::None))
        .round(0, all(LtpReq::Lfsr1))
        .frl_start_with_round(2)
        .round(0, all(LtpReq::None))
        .frl_start_after(0)
}

#[test]
fn frl_start_with_a_retrain_request_retrains() {
    let (outcome, sink, phy) = run(both_flags_sink(), &[RATE], &TrainingConfig::default());
    assert_eq!(
        outcome,
        TrainingOutcome::Success {
            achieved_rate: RATE
        }
    );
    assert_eq!(retrain_count(&phy), 1);
    // FRL_start was cleared before retraining, so it could not end the next LTS:P.
    let clears: Vec<_> = sink
        .calls
        .iter()
        .filter(|c| **c == SinkCall::ClearUpdateFlags(FRL_START))
        .collect();
    assert_eq!(clears.len(), 2);
    assert!(ltp_sent(&phy).contains(&uniform(4, Some(LtpPattern::Lfsr1))));
}

#[test]
fn frl_start_with_a_retrain_request_past_max_retrains_falls_back() {
    let config = TrainingConfig {
        max_retrains: 0,
        ..TrainingConfig::default()
    };
    let (outcome, _, _) = run(both_flags_sink(), &[RATE], &config);
    assert_eq!(outcome, fallback(FallbackReason::RetrainsExhausted));
}

fn retraining_sink(retrains: usize) -> SimSink {
    let mut sink = SimSink::new()
        .flt_ready_after(0)
        .round(0, all(LtpReq::None));
    for _ in 0..retrains {
        sink = sink.round(0, all(LtpReq::None));
    }
    sink.frl_start_after(0)
}

fn retrain_count(phy: &SimPhy) -> usize {
    // Each pass through LTS:P sets gap-only output once.
    phy.calls
        .iter()
        .filter(|c| **c == PhyCall::SetFrlOutput(FrlOutput::GapOnly))
        .count()
        - 2
}

#[test]
fn retrains_up_to_max_retrains_then_falls_back() {
    let config = TrainingConfig {
        max_retrains: 3,
        ..TrainingConfig::default()
    };
    // Three retrains are allowed; the fourth request ends the attempt.
    let (outcome, sink, phy) = run(retraining_sink(4), &[RATE], &config);
    assert_eq!(outcome, fallback(FallbackReason::RetrainsExhausted));
    assert_eq!(retrain_count(&phy), 3);
    assert_eq!(phy_rates(&phy).last(), Some(&HdmiForumFrl::NotSupported));
    assert_eq!(frl_configs(&sink).last(), Some(&HdmiForumFrl::NotSupported));
}

#[test]
fn exhausted_retrains_fall_back_under_flt_no_timeout_too() {
    // The retrain bound is not a timer: FLT_no_timeout does not turn it into a hold.
    let config = TrainingConfig {
        max_retrains: 1,
        ..TrainingConfig::default()
    };
    let sink = retraining_sink(2).source_test(NO_TIMEOUT);
    let (outcome, sink, phy) = run(sink, &[RATE], &config);
    assert_eq!(outcome, fallback(FallbackReason::RetrainsExhausted));
    assert_eq!(phy_rates(&phy).last(), Some(&HdmiForumFrl::NotSupported));
    assert_eq!(frl_configs(&sink).last(), Some(&HdmiForumFrl::NotSupported));
}

#[test]
fn retrains_within_max_retrains_succeed() {
    let config = TrainingConfig {
        max_retrains: 3,
        ..TrainingConfig::default()
    };
    let (outcome, _, phy) = run(retraining_sink(3), &[RATE], &config);
    assert_eq!(
        outcome,
        TrainingOutcome::Success {
            achieved_rate: RATE
        }
    );
    assert_eq!(retrain_count(&phy), 3);
}

#[test]
fn max_retrains_0_falls_back_on_the_first_request() {
    let config = TrainingConfig {
        max_retrains: 0,
        ..TrainingConfig::default()
    };
    let (outcome, _, phy) = run(retraining_sink(1), &[RATE], &config);
    assert_eq!(outcome, fallback(FallbackReason::RetrainsExhausted));
    assert_eq!(retrain_count(&phy), 0);
}

#[test]
fn the_retrain_budget_covers_the_whole_call() {
    // One retrain at 12 Gbps uses the budget; after stepping down to 10 Gbps, the next
    // request exhausts it.
    let config = TrainingConfig {
        max_retrains: 1,
        ..TrainingConfig::default()
    };
    let sink = SimSink::new()
        .flt_ready_after(0)
        .round(0, all(LtpReq::None))
        .round(0, all(LtpReq::RateChange))
        .round(0, all(LtpReq::None))
        .round(0, all(LtpReq::None))
        .frl_start_after(0);
    let (outcome, sink, _) = run(sink, &[R12, R10], &config);
    assert_eq!(outcome, fallback(FallbackReason::RetrainsExhausted));
    assert_eq!(frl_configs(&sink), [R12, R10, HdmiForumFrl::NotSupported]);
}

// --- Errors

/// A sink that exercises every SCDC operation the state machine uses.
fn full_sink() -> SimSink {
    SimSink::new()
        .source_test(SourceTestConfig::default())
        .flt_ready_after(0)
        .round(0, all(LtpReq::Lfsr0))
        .round(0, all(LtpReq::None))
        .frl_start_after(0)
}

/// Asserts that the last `Config_1` written and the last PHY rate are TMDS.
fn assert_in_tmds(sink: &SimSink, phy: &SimPhy) {
    assert_eq!(frl_configs(sink).last(), Some(&HdmiForumFrl::NotSupported));
    assert_eq!(phy_rates(phy).last(), Some(&HdmiForumFrl::NotSupported));
}

#[test]
fn scdc_errors_are_returned_after_lts_l() {
    for op in [
        SinkOp::ReadFltReady,
        SinkOp::ReadUpdateFlags,
        SinkOp::ClearUpdateFlags,
        SinkOp::ReadLtpRequests,
        SinkOp::ReadSourceTestConfig,
        SinkOp::WriteConfig0Defaults,
        SinkOp::WriteFrlConfig,
    ] {
        let (result, sink, phy) = run_with(full_sink().fail_call(op, 1), SimPhy::new());
        let exit = TmdsExit::Exited;
        assert_eq!(
            result,
            Err(TrainingError::Scdc { error: (), exit }),
            "{op:?}"
        );
        assert_in_tmds(&sink, &phy);
    }
}

#[test]
fn phy_errors_are_returned_after_lts_l() {
    for op in [
        PhyOp::SetFrlRate,
        PhyOp::SendLtp,
        PhyOp::SetFrlOutput,
        PhyOp::AdjustEqualization,
    ] {
        let (result, sink, phy) = run_with(full_sink(), SimPhy::new().fail_call(op, 1));
        let exit = TmdsExit::Exited;
        assert_eq!(
            result,
            Err(TrainingError::Phy { error: (), exit }),
            "{op:?}"
        );
        assert_in_tmds(&sink, &phy);
    }
}

#[test]
fn a_failed_lts_l_after_an_error_reports_each_end() {
    // Every `Config_1` write fails, so LTS:L's does too.
    let (result, _, phy) = run_with(full_sink().fail(SinkOp::WriteFrlConfig), SimPhy::new());
    let exit = TmdsExit::Failed(exit_error(true, false));
    assert_eq!(result, Err(TrainingError::Scdc { error: (), exit }));
    assert_eq!(phy_rates(&phy).last(), Some(&HdmiForumFrl::NotSupported));

    // Every PHY rate change fails, so LTS:L's does too.
    let (result, sink, _) = run_with(full_sink(), SimPhy::new().fail(PhyOp::SetFrlRate));
    let exit = TmdsExit::Failed(exit_error(false, true));
    assert_eq!(result, Err(TrainingError::Phy { error: (), exit }));
    assert_eq!(frl_configs(&sink).last(), Some(&HdmiForumFrl::NotSupported));
}

#[test]
fn exit_to_tmds_on_error_off_leaves_both_ends_as_they_were() {
    let config = TrainingConfig {
        exit_to_tmds_on_error: false,
        ..TrainingConfig::default()
    };
    let mut trainer = FrlTrainer::new(
        full_sink().fail_call(SinkOp::ReadLtpRequests, 1),
        SimPhy::new(),
    );
    let result = trainer.train(&[RATE], &config);
    let exit = TmdsExit::Skipped;
    assert_eq!(result, Err(TrainingError::Scdc { error: (), exit }));
    let (sink, phy) = trainer.into_parts();
    assert_eq!(frl_configs(&sink), [RATE]);
    assert_eq!(phy_rates(&phy), [RATE]);
}

// --- exit_to_tmds on demand

#[test]
fn exit_to_tmds_performs_lts_l() {
    let mut trainer = FrlTrainer::new(SimSink::new(), SimPhy::new());
    assert_eq!(trainer.exit_to_tmds(), Ok(()));
    let (sink, phy) = trainer.into_parts();
    assert_eq!(
        phy.calls,
        [
            PhyCall::SendLtp(LanePatterns::default()),
            PhyCall::SetFrlRate(HdmiForumFrl::NotSupported),
        ]
    );
    assert_eq!(
        sink.calls,
        [
            SinkCall::WriteFrlConfig(FrlConfig {
                rate: HdmiForumFrl::NotSupported,
                ffe_levels: FfeLevels::default(),
            }),
            SinkCall::ReadUpdateFlags(NO_FLAGS),
        ]
    );
}

#[test]
fn exit_to_tmds_takes_down_a_link_left_by_an_error() {
    let config = TrainingConfig {
        exit_to_tmds_on_error: false,
        ..TrainingConfig::default()
    };
    let mut trainer = FrlTrainer::new(
        full_sink().fail_call(SinkOp::ReadLtpRequests, 1),
        SimPhy::new(),
    );
    assert!(trainer.train(&[RATE], &config).is_err());
    assert_eq!(trainer.exit_to_tmds(), Ok(()));
    let (sink, phy) = trainer.into_parts();
    assert_in_tmds(&sink, &phy);
}

#[test]
fn exit_to_tmds_reports_each_ends_error() {
    let mut trainer = FrlTrainer::new(
        SimSink::new().fail(SinkOp::ReadUpdateFlags),
        SimPhy::new().fail(PhyOp::SendLtp),
    );
    assert_eq!(trainer.exit_to_tmds(), Err(exit_error(true, true)));
    // The failed steps did not stop the others.
    let (sink, phy) = trainer.into_parts();
    assert_in_tmds(&sink, &phy);
}

#[test]
fn lts_exit_to_tmds_records_its_event() {
    let (mut scdc, mut phy) = (SimSink::new(), SimPhy::new().fail(PhyOp::SetFrlRate));
    let mut io = SyncIo {
        scdc: &mut scdc,
        phy: &mut phy,
    };
    let mut events = Vec::new();
    let result = block_on(crate::lts::exit_to_tmds(&mut io, &mut |event| {
        events.push(event)
    }));
    assert_eq!(result, Err(exit_error(false, true)));
    assert_eq!(
        events,
        [TrainingEvent::ExitToTmdsFailed {
            scdc: false,
            phy: true
        }]
    );
}

#[test]
fn train_with_events_reports_each_event_as_it_occurs() {
    let sink = SimSink::new()
        .flt_ready_after(0)
        .round(0, all(LtpReq::Lfsr0))
        .round(0, all(LtpReq::None))
        .frl_start_after(0);
    let mut events = Vec::new();
    let trained = FrlTrainer::new(sink, SimPhy::new())
        .train_with_events(&[RATE], &TrainingConfig::default(), &mut |event| {
            events.push(event)
        })
        .unwrap();
    assert_eq!(
        trained.outcome,
        TrainingOutcome::Success {
            achieved_rate: RATE
        }
    );
    assert_eq!(
        events[..2],
        [
            TrainingEvent::SourceTestConfigRead {
                flt_no_timeout: false
            },
            TrainingEvent::FltReady { after_polls: 1 },
        ]
    );
    assert!(matches!(
        events.last(),
        Some(TrainingEvent::FrlStart { .. })
    ));
}

#[test]
fn lts_l_after_an_error_is_recorded() {
    let events = |sink: SimSink, phy: SimPhy| {
        let mut events = Vec::new();
        let mut trainer = FrlTrainer::new(sink, phy);
        let _ = trainer.train_with_events(&[RATE], &TrainingConfig::default(), &mut |event| {
            events.push(event)
        });
        events.last().copied()
    };
    assert_eq!(
        events(
            full_sink().fail_call(SinkOp::ReadLtpRequests, 1),
            SimPhy::new()
        ),
        Some(TrainingEvent::ExitedToTmds)
    );
    assert_eq!(
        events(full_sink(), SimPhy::new().fail(PhyOp::SendLtp)),
        Some(TrainingEvent::ExitToTmdsFailed {
            scdc: false,
            phy: true
        })
    );
    assert_eq!(
        events(full_sink().fail(SinkOp::WriteFrlConfig), SimPhy::new()),
        Some(TrainingEvent::ExitToTmdsFailed {
            scdc: true,
            phy: false
        })
    );
}

// --- Traces

#[cfg(feature = "alloc")]
mod traced {
    use super::*;
    use crate::trace::{TrainingEvent, TrainingTrace};

    fn trace(
        sink: SimSink,
        rates: &[HdmiForumFrl],
        config: &TrainingConfig,
    ) -> (TrainingOutcome, TrainingTrace) {
        let (result, trace) = FrlTrainer::new(sink, SimPhy::new()).train_traced(rates, config);
        (result.unwrap().outcome, trace)
    }

    /// The first example trace in the architecture doc.
    #[test]
    fn documented_trace_of_a_successful_attempt() {
        let sink = SimSink::new()
            .flt_ready_after(2)
            .round(
                0,
                requests(LtpReq::Lfsr0, LtpReq::Lfsr1, LtpReq::Lfsr2, LtpReq::Lfsr3),
            )
            .round(
                0,
                requests(LtpReq::None, LtpReq::FfeChange, LtpReq::None, LtpReq::None),
            )
            .round(38, all(LtpReq::None))
            .frl_start_after(5);
        let (outcome, trace) = trace(sink, &[R12], &TrainingConfig::default());
        assert_eq!(outcome, TrainingOutcome::Success { achieved_rate: R12 });
        assert_eq!(
            trace.events,
            [
                TrainingEvent::SourceTestConfigRead {
                    flt_no_timeout: false,
                },
                TrainingEvent::FltReady { after_polls: 3 },
                TrainingEvent::RateConfigured {
                    rate: R12,
                    ffe_levels: FfeLevels::new(3).unwrap(),
                },
                TrainingEvent::LtpRequested {
                    requests: requests(LtpReq::Lfsr0, LtpReq::Lfsr1, LtpReq::Lfsr2, LtpReq::Lfsr3),
                },
                TrainingEvent::LtpRequested {
                    requests: requests(LtpReq::None, LtpReq::FfeChange, LtpReq::None, LtpReq::None),
                },
                TrainingEvent::FfeRaised { lane: 1, level: 1 },
                TrainingEvent::TrainingPassed { after_polls: 41 },
                TrainingEvent::FrlStart { after_polls: 6 },
            ]
        );
    }

    /// The second example trace in the architecture doc: the sink asks for a lower rate.
    #[test]
    fn documented_trace_of_a_rate_drop() {
        let sink = SimSink::new()
            .flt_ready_after(1)
            .round(0, all(LtpReq::RateChange))
            .round(5, all(LtpReq::Lfsr0))
            .round(5, all(LtpReq::None))
            .frl_start_after(3);
        let (outcome, trace) = trace(sink, &[R12, R10], &TrainingConfig::default());
        assert_eq!(outcome, TrainingOutcome::Success { achieved_rate: R10 });
        let three = FfeLevels::new(3).unwrap();
        assert_eq!(
            trace.events,
            [
                TrainingEvent::SourceTestConfigRead {
                    flt_no_timeout: false,
                },
                TrainingEvent::FltReady { after_polls: 2 },
                TrainingEvent::RateConfigured {
                    rate: R12,
                    ffe_levels: three,
                },
                TrainingEvent::LtpRequested {
                    requests: all(LtpReq::RateChange),
                },
                TrainingEvent::RateLowered { from: R12, to: R10 },
                TrainingEvent::RateConfigured {
                    rate: R10,
                    ffe_levels: three,
                },
                TrainingEvent::LtpRequested {
                    requests: all(LtpReq::Lfsr0),
                },
                TrainingEvent::TrainingPassed { after_polls: 12 },
                TrainingEvent::FrlStart { after_polls: 4 },
            ]
        );
    }

    #[test]
    fn timeouts_record_the_limit_and_end_in_tmds() {
        let config = TrainingConfig {
            flt_ready_polls: 4,
            ltp_polls: 5,
            frl_start_polls: 6,
            ..TrainingConfig::default()
        };
        let cases = [
            (SimSink::new(), TrainingEvent::FltReadyTimeout { polls: 4 }),
            (
                SimSink::new().flt_ready_after(0),
                TrainingEvent::TrainingTimeout { polls: 5 },
            ),
            (
                SimSink::new()
                    .flt_ready_after(0)
                    .round(0, all(LtpReq::None)),
                TrainingEvent::FrlStartTimeout { polls: 6 },
            ),
        ];
        for (sink, timeout) in cases {
            let (_, trace) = trace(sink, &[RATE], &config);
            assert_eq!(
                trace.events[trace.events.len() - 2..],
                [timeout, TrainingEvent::ExitedToTmds]
            );
        }
    }

    #[test]
    fn exhausted_rates_end_in_tmds() {
        let sink = SimSink::new()
            .flt_ready_after(0)
            .round(0, all(LtpReq::RateChange));
        let (_, trace) = trace(sink, &[RATE], &TrainingConfig::default());
        assert_eq!(
            trace.events[trace.events.len() - 3..],
            [
                TrainingEvent::LtpRequested {
                    requests: all(LtpReq::RateChange),
                },
                TrainingEvent::RatesExhausted,
                TrainingEvent::ExitedToTmds,
            ]
        );
    }

    #[test]
    fn exhausted_retrains_end_in_tmds() {
        let config = TrainingConfig {
            max_retrains: 1,
            ..TrainingConfig::default()
        };
        let (_, trace) = trace(retraining_sink(2), &[RATE], &config);
        let tail: Vec<_> = trace
            .events
            .iter()
            .filter(|e| {
                matches!(
                    e,
                    TrainingEvent::RetrainRequested
                        | TrainingEvent::RetrainsExhausted { .. }
                        | TrainingEvent::ExitedToTmds
                )
            })
            .collect();
        assert_eq!(
            tail,
            [
                &TrainingEvent::RetrainRequested,
                &TrainingEvent::RetrainsExhausted { retrains: 1 },
                &TrainingEvent::ExitedToTmds,
            ]
        );
    }

    #[test]
    fn source_test_reads_and_retrains_are_recorded() {
        let sink = SimSink::new()
            .source_test(SourceTestConfig::default())
            .flt_ready_after(0)
            .round(0, all(LtpReq::None))
            .round_with_source_test(0, all(LtpReq::Lfsr0), NO_TIMEOUT)
            .round(0, all(LtpReq::None))
            .frl_start_after(0);
        let (_, trace) = trace(sink, &[RATE], &TrainingConfig::default());
        let notable: Vec<_> = trace
            .events
            .iter()
            .filter(|e| {
                matches!(
                    e,
                    TrainingEvent::SourceTestConfigRead { .. } | TrainingEvent::RetrainRequested
                )
            })
            .collect();
        assert_eq!(
            notable,
            [
                &TrainingEvent::SourceTestConfigRead {
                    flt_no_timeout: false
                },
                &TrainingEvent::RetrainRequested,
                &TrainingEvent::SourceTestConfigRead {
                    flt_no_timeout: true
                },
            ]
        );
    }

    #[test]
    fn a_held_ffe_level_is_not_recorded_as_raised() {
        let config = TrainingConfig {
            ffe_levels: FfeLevels::new(1).unwrap(),
            ..TrainingConfig::default()
        };
        let sink = SimSink::new()
            .flt_ready_after(0)
            .round(0, all(LtpReq::FfeChange))
            .round(0, all(LtpReq::FfeChange))
            .round(0, all(LtpReq::None))
            .frl_start_after(0);
        let (_, trace) = trace(sink, &[RATE], &config);
        let raised = trace
            .events
            .iter()
            .filter(|e| matches!(e, TrainingEvent::FfeRaised { .. }))
            .count();
        assert_eq!(raised, 4);
    }

    #[test]
    fn reserved_requests_are_recorded_on_every_lane() {
        let rate = HdmiForumFrl::Rate6Gbps3Lanes;
        let sink = SimSink::new()
            .flt_ready_after(0)
            .round(
                0,
                requests(
                    LtpReq::Lfsr0,
                    LtpReq::Reserved(0xC),
                    LtpReq::Lfsr0,
                    LtpReq::Reserved(0x9),
                ),
            )
            .round(0, all(LtpReq::None))
            .frl_start_after(0);
        let (_, trace) = trace(sink, &[rate], &TrainingConfig::default());
        let undefined: Vec<_> = trace
            .events
            .iter()
            .filter(|e| matches!(e, TrainingEvent::UndefinedLtpRequest { .. }))
            .collect();
        // Lane 3 is not in use at a 3-lane rate, and is recorded as such.
        assert_eq!(
            undefined,
            [
                &TrainingEvent::UndefinedLtpRequest {
                    lane: 1,
                    value: 0xC,
                    in_use: true
                },
                &TrainingEvent::UndefinedLtpRequest {
                    lane: 3,
                    value: 0x9,
                    in_use: false
                },
            ]
        );
    }

    #[test]
    fn the_no_timeout_cap_is_recorded_instead_of_a_timeout() {
        let config = TrainingConfig {
            no_timeout_poll_cap: 3,
            ..TrainingConfig::default()
        };
        let sink = SimSink::new()
            .source_test(NO_TIMEOUT)
            .flt_ready_after(0)
            .round(0, all(LtpReq::None));
        let (outcome, trace) = trace(sink, &[RATE], &config);
        assert_eq!(outcome, TrainingOutcome::NoTimeoutHold { rate: RATE });
        assert_eq!(
            trace.events.last(),
            Some(&TrainingEvent::NoTimeoutCapReached { polls: 3 })
        );
        assert!(!trace.events.contains(&TrainingEvent::ExitedToTmds));
    }

    #[test]
    fn frl_start_with_a_retrain_is_recorded() {
        let (_, trace) = trace(both_flags_sink(), &[RATE], &TrainingConfig::default());
        let at = trace
            .events
            .iter()
            .position(|e| *e == TrainingEvent::FrlStartWithRetrain)
            .expect("recorded");
        assert_eq!(trace.events[at + 1], TrainingEvent::RetrainRequested);
    }

    #[test]
    fn the_trace_carries_the_rates_and_config() {
        let config = TrainingConfig::default();
        let (_, trace) = trace(SimSink::new(), &[R12, R10], &config);
        assert_eq!(trace.rates, [R12, R10]);
        assert_eq!(trace.config, config);
    }

    #[test]
    fn train_with_events_delivers_the_traced_events() {
        let sink = || {
            SimSink::new()
                .source_test(SourceTestConfig::default())
                .flt_ready_after(2)
                .round(0, all(LtpReq::Lfsr1))
                .round(0, all(LtpReq::RateChange))
                .round(0, all(LtpReq::None))
                .frl_start_after(1)
        };
        let rates = [RATE, HdmiForumFrl::Rate3Gbps3Lanes];
        let config = TrainingConfig::default();
        let (_, trace) = trace(sink(), &rates, &config);
        let mut events = Vec::new();
        let _ = FrlTrainer::new(sink(), SimPhy::new()).train_with_events(
            &rates,
            &config,
            &mut |event| events.push(event),
        );
        assert_eq!(events, trace.events);
    }

    #[test]
    fn traced_and_untraced_outcomes_match() {
        let sink = || {
            SimSink::new()
                .flt_ready_after(0)
                .round(0, all(LtpReq::Lfsr1))
                .round(0, all(LtpReq::None))
                .frl_start_after(0)
        };
        let mut trainer = FrlTrainer::new(sink(), SimPhy::new());
        let (outcome, trace) = trainer.train_at_rate_traced(RATE, &TrainingConfig::default());
        let (untraced, _, _) = run(sink(), &[RATE], &TrainingConfig::default());
        assert_eq!(outcome.map(|trained| trained.outcome), Ok(untraced));
        assert_eq!(trace.rates, [RATE]);
    }

    #[test]
    fn an_empty_rate_list_has_no_events() {
        let (result, trace) = FrlTrainer::new(SimSink::new(), SimPhy::new())
            .train_traced(&[], &TrainingConfig::default());
        assert_eq!(result, Err(TrainingError::NoRates));
        assert!(trace.events.is_empty());
        assert!(trace.rates.is_empty());
    }

    #[test]
    fn traced_errors_are_returned_with_the_trace() {
        let mut trainer =
            FrlTrainer::new(SimSink::new().fail(SinkOp::ReadUpdateFlags), SimPhy::new());
        let (result, trace) = trainer.train_traced(&[RATE], &TrainingConfig::default());
        let exit = TmdsExit::Failed(exit_error(true, false));
        assert_eq!(result, Err(TrainingError::Scdc { error: (), exit }));
        assert_eq!(trace.rates, [RATE]);
        assert_eq!(
            trace.events,
            [TrainingEvent::ExitToTmdsFailed {
                scdc: true,
                phy: false
            }]
        );
    }

    #[test]
    fn an_error_trace_records_the_attempt_up_to_the_error() {
        let sink = SimSink::new()
            .flt_ready_after(2)
            .fail_call(SinkOp::ReadLtpRequests, 1)
            .round(0, all(LtpReq::Lfsr0));
        let (result, trace) =
            FrlTrainer::new(sink, SimPhy::new()).train_traced(&[RATE], &TrainingConfig::default());
        let exit = TmdsExit::Exited;
        assert_eq!(result, Err(TrainingError::Scdc { error: (), exit }));
        assert_eq!(
            trace.events,
            [
                TrainingEvent::SourceTestConfigRead {
                    flt_no_timeout: false,
                },
                TrainingEvent::FltReady { after_polls: 3 },
                TrainingEvent::RateConfigured {
                    rate: RATE,
                    ffe_levels: FfeLevels::new(3).unwrap()
                },
                TrainingEvent::ExitedToTmds,
            ]
        );
    }

    #[test]
    fn training_trace_new_sets_fields() {
        let events = Vec::from([TrainingEvent::ExitedToTmds]);
        let trace =
            TrainingTrace::new(Vec::from([RATE]), TrainingConfig::default(), events.clone());
        assert_eq!(trace.rates, [RATE]);
        assert_eq!(trace.config, TrainingConfig::default());
        assert_eq!(trace.events, events);
    }
}

// --- The sync driver

#[test]
#[should_panic(expected = "plumbob's sync training waited on I/O")]
fn the_sync_driver_rejects_a_future_that_waits() {
    // The same output type as the trainer tests' runs, so the check is on that path.
    let _ = ready::<Result<Trained, TrainingError<(), ()>>>(Poll::Pending);
}
