extern crate std;
use std::vec::Vec;

use super::*;
use crate::lts::sim::{PhyCall, PhyOp, SimPhy, SimSink, SinkCall, SinkOp, all};
use crate::lts::types::SourceTestConfig;

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
    let outcome = trainer.train(rates, config).unwrap();
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
    assert_eq!(config.ffe_levels, FfeLevels::default());
    assert_eq!(config.flt_ready_polls, 50);
    assert_eq!(config.ltp_polls, 100);
    assert_eq!(config.frl_start_polls, 125);
    assert_eq!(config.no_timeout_poll_cap, 100_000);
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
            SinkCall::ReadFltReady(false),
            SinkCall::ReadFltReady(true),
            SinkCall::ClearUpdateFlags(flt_update),
            SinkCall::WriteConfig0Defaults,
            SinkCall::WriteFrlConfig(FrlConfig {
                rate: RATE,
                ffe_levels: FfeLevels::default(),
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
        .unwrap();
    let (at_rate, _) = trainer.into_parts();
    let (expected, list, _) = run(sink(), &[RATE], &TrainingConfig::default());
    assert_eq!(outcome, expected);
    assert_eq!(at_rate.calls, list.calls);
}

#[test]
fn empty_rate_list_does_not_touch_the_sink() {
    let (outcome, sink, phy) = run(SimSink::new(), &[], &TrainingConfig::default());
    assert_eq!(outcome, fallback(FallbackReason::RatesExhausted));
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
fn flt_no_timeout_is_capped() {
    let config = TrainingConfig {
        no_timeout_poll_cap: 3,
        ..TrainingConfig::default()
    };
    let sink = SimSink::new().source_test(NO_TIMEOUT);
    let (outcome, sink, _) = run(sink, &[RATE], &config);
    assert_eq!(outcome, fallback(FallbackReason::FltReadyTimeout));
    assert_eq!(count(&sink, |c| matches!(c, SinkCall::ReadFltReady(_))), 3);
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
    assert_eq!(
        count(&sink, |c| matches!(c, SinkCall::ReadSourceTestConfig(_))),
        1
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

#[test]
fn scdc_errors_are_returned() {
    for op in [
        SinkOp::ReadFltReady,
        SinkOp::ReadUpdateFlags,
        SinkOp::ClearUpdateFlags,
        SinkOp::ReadLtpRequests,
        SinkOp::ReadSourceTestConfig,
        SinkOp::WriteConfig0Defaults,
        SinkOp::WriteFrlConfig,
    ] {
        let mut trainer = FrlTrainer::new(full_sink().fail(op), SimPhy::new());
        let result = trainer.train(&[RATE], &TrainingConfig::default());
        assert_eq!(result, Err(TrainingError::Scdc(())), "{op:?}");
    }
}

#[test]
fn phy_errors_are_returned() {
    for op in [
        PhyOp::SetFrlRate,
        PhyOp::SendLtp,
        PhyOp::SetFrlOutput,
        PhyOp::AdjustEqualization,
    ] {
        let mut trainer = FrlTrainer::new(full_sink(), SimPhy::new().fail(op));
        let result = trainer.train(&[RATE], &TrainingConfig::default());
        assert_eq!(result, Err(TrainingError::Phy(())), "{op:?}");
    }
}

// --- Traces

#[cfg(feature = "alloc")]
mod traced {
    use super::*;
    use crate::lts::trace::{TrainingEvent, TrainingTrace};

    fn trace(
        sink: SimSink,
        rates: &[HdmiForumFrl],
        config: &TrainingConfig,
    ) -> (TrainingOutcome, TrainingTrace) {
        FrlTrainer::new(sink, SimPhy::new())
            .train_traced(rates, config)
            .unwrap()
    }

    fn ffe_3() -> TrainingConfig {
        TrainingConfig {
            ffe_levels: FfeLevels::new(3).unwrap(),
            ..TrainingConfig::default()
        }
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
        let (outcome, trace) = trace(sink, &[R12], &ffe_3());
        assert_eq!(outcome, TrainingOutcome::Success { achieved_rate: R12 });
        assert_eq!(
            trace.events,
            [
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
        let (outcome, trace) = trace(sink, &[R12, R10], &ffe_3());
        assert_eq!(outcome, TrainingOutcome::Success { achieved_rate: R10 });
        let three = FfeLevels::new(3).unwrap();
        assert_eq!(
            trace.events,
            [
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
    fn the_trace_carries_the_rates_and_config() {
        let config = ffe_3();
        let (_, trace) = trace(SimSink::new(), &[R12, R10], &config);
        assert_eq!(trace.rates, [R12, R10]);
        assert_eq!(trace.config, config);
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
        let (outcome, trace) = trainer
            .train_at_rate_traced(RATE, &TrainingConfig::default())
            .unwrap();
        let (untraced, _, _) = run(sink(), &[RATE], &TrainingConfig::default());
        assert_eq!(outcome, untraced);
        assert_eq!(trace.rates, [RATE]);
    }

    #[test]
    fn an_empty_rate_list_has_no_events() {
        let (outcome, trace) = trace(SimSink::new(), &[], &TrainingConfig::default());
        assert_eq!(outcome, fallback(FallbackReason::RatesExhausted));
        assert!(trace.events.is_empty());
    }

    #[test]
    fn traced_errors_are_returned() {
        let mut trainer =
            FrlTrainer::new(SimSink::new().fail(SinkOp::ReadUpdateFlags), SimPhy::new());
        assert_eq!(
            trainer.train_traced(&[RATE], &TrainingConfig::default()),
            Err(TrainingError::Scdc(()))
        );
    }

    #[test]
    fn training_trace_new_sets_fields() {
        let events = Vec::from([TrainingEvent::ExitedToTmds]);
        let trace = TrainingTrace::new(Vec::from([RATE]), ffe_3(), events.clone());
        assert_eq!(trace.rates, [RATE]);
        assert_eq!(trace.config, ffe_3());
        assert_eq!(trace.events, events);
    }
}
