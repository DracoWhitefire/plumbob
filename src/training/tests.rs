extern crate std;

use display_types::cea861::hdmi_forum::HdmiForumFrl;

use crate::types::{FfeLevels, LtpReq};

use super::sim::{MockPhy, RATE, SimScdc, cfg, flt_ready, frl_started, ltp, not_ready};
use super::*;

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
// SimScdc and MockPhy behaviour
// -------------------------------------------------------------------------

#[test]
fn sim_scdc_empty_queue_returns_err() {
    use crate::scdc::ScdcClient;
    // No statuses pushed — the exhausted-queue fallback must return Err(()).
    let mut scdc = SimScdc::new();
    assert_eq!(scdc.read_training_status(), Err(()));
}

#[test]
fn sim_scdc_statuses_consumed_fifo() {
    use crate::scdc::ScdcClient;
    let mut scdc = SimScdc::new();
    scdc.push(not_ready());
    scdc.push(flt_ready());
    assert_eq!(scdc.read_training_status(), Ok(not_ready()));
    assert_eq!(scdc.read_training_status(), Ok(flt_ready()));
    assert_eq!(scdc.read_training_status(), Err(())); // queue now empty
}

#[test]
fn sim_scdc_ced_calls_counted() {
    // Two LTP iterations → read_ced called exactly twice.
    let mut scdc = SimScdc::new();
    scdc.push(flt_ready());
    scdc.push(frl_started());
    scdc.push(ltp(LtpReq::Lfsr0));
    scdc.push(ltp(LtpReq::Lfsr1));
    scdc.push(frl_started()); // ltp_req = None → success

    let mut trainer = FrlTrainer::new(scdc, MockPhy::new());
    trainer
        .train_at_rate(RATE, &TrainingConfig::default())
        .unwrap();
    let (scdc, _) = trainer.into_parts();
    assert_eq!(scdc.ced_calls, 2);
}

#[test]
fn sim_scdc_read_ced_returns_all_none() {
    use crate::scdc::ScdcClient;
    let mut scdc = SimScdc::new();
    let ced = scdc.read_ced().unwrap();
    assert!(ced.lane0.is_none());
    assert!(ced.lane1.is_none());
    assert!(ced.lane2.is_none());
    assert!(ced.lane3.is_none());
}

#[test]
fn mock_phy_adjust_equalization_always_succeeds() {
    use hdmi_hal::phy::{EqParams, HdmiPhy};
    let mut phy = MockPhy::new();
    assert!(phy.adjust_equalization(EqParams::default()).is_ok());
}

#[test]
fn mock_phy_set_scrambling_always_succeeds() {
    use hdmi_hal::phy::HdmiPhy;
    let mut phy = MockPhy::new();
    assert!(phy.set_scrambling(true).is_ok());
    assert!(phy.set_scrambling(false).is_ok());
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

    use super::*;

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

    #[test]
    fn training_trace_new_sets_fields() {
        extern crate std;
        use crate::trace::TrainingTrace;
        use display_types::cea861::hdmi_forum::HdmiForumFrl;
        use std::vec;

        let rate = HdmiForumFrl::Rate6Gbps4Lanes;
        let config = TrainingConfig::default();
        let events = vec![
            TrainingEvent::RateConfigured {
                rate,
                ffe_levels: FfeLevels::Ffe0,
            },
            TrainingEvent::AllLanesSatisfied {
                after_iterations: 3,
            },
        ];
        let trace = TrainingTrace::new(rate, config, events.clone());
        assert_eq!(trace.rate, rate);
        assert_eq!(trace.config, config);
        assert_eq!(trace.events, events);
    }
}
