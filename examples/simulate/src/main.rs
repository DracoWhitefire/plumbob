//! Simulated FRL training example for `plumbob`.
//!
//! Demonstrates the full `FrlTrainer` usage pattern against in-memory
//! implementations of `ScdcClient` and `HdmiPhy`. No hardware required.
//!
//! Run with `cargo run` from this directory.

use core::convert::Infallible;
use display_types::cea861::hdmi_forum::HdmiForumFrl;
use hdmi_hal::phy::{EqParams, HdmiPhy, LtpPattern};
use plumbob::{
    CedCounters, FrlConfig, FrlTrainer, LtpReq, ScdcClient, TrainingConfig, TrainingStatus,
};

// --- SimScdc ---------------------------------------------------------------------
//
// Scripted ScdcClient backed by a call counter. Returns a pre-set sequence of
// TrainingStatus values that steers the state machine through all four phases:
//
//   Phase 2 — flt_ready asserts after 3 failed polls.
//   Phase 3 — frl_start asserts after 5 failed polls.
//   Phase 4 — ltp_req transitions Lfsr0 → Lfsr2 → None over 4 iterations.

struct SimScdc {
    call: u32,
}

impl SimScdc {
    fn new() -> Self {
        Self { call: 0 }
    }
}

impl ScdcClient for SimScdc {
    type Error = Infallible;

    fn write_frl_config(&mut self, config: FrlConfig) -> Result<(), Infallible> {
        println!(
            "SCDC: write_frl_config(rate={:?}, ffe_levels={:?}, dsc_frl_max={})",
            config.rate, config.ffe_levels, config.dsc_frl_max
        );
        Ok(())
    }

    fn read_training_status(&mut self) -> Result<TrainingStatus, Infallible> {
        self.call += 1;
        let status = match self.call {
            // Phase 2: 3 failed polls then flt_ready.
            1..=3 => TrainingStatus {
                flt_ready: false,
                frl_start: false,
                ltp_req: LtpReq::None,
            },
            4 => TrainingStatus {
                flt_ready: true,
                frl_start: false,
                ltp_req: LtpReq::None,
            },
            // Phase 3: 5 failed polls then frl_start.
            5..=9 => TrainingStatus {
                flt_ready: true,
                frl_start: false,
                ltp_req: LtpReq::None,
            },
            10 => TrainingStatus {
                flt_ready: true,
                frl_start: true,
                ltp_req: LtpReq::None,
            },
            // Phase 4: Lfsr0 for 2 iterations, Lfsr2 for 1, then None.
            11..=12 => TrainingStatus {
                flt_ready: true,
                frl_start: true,
                ltp_req: LtpReq::Lfsr0,
            },
            13 => TrainingStatus {
                flt_ready: true,
                frl_start: true,
                ltp_req: LtpReq::Lfsr2,
            },
            _ => TrainingStatus {
                flt_ready: true,
                frl_start: true,
                ltp_req: LtpReq::None,
            },
        };
        Ok(status)
    }

    fn read_ced(&mut self) -> Result<CedCounters, Infallible> {
        Ok(CedCounters {
            lane0: None,
            lane1: None,
            lane2: None,
            lane3: None,
        })
    }
}

// --- SimPhy ----------------------------------------------------------------------

struct SimPhy;

impl HdmiPhy for SimPhy {
    type Error = Infallible;

    fn set_frl_rate(&mut self, rate: HdmiForumFrl) -> Result<(), Infallible> {
        println!("PHY:  set_frl_rate({rate:?})");
        Ok(())
    }

    fn send_ltp(&mut self, pattern: LtpPattern) -> Result<(), Infallible> {
        println!("PHY:  send_ltp({})", pattern.value());
        Ok(())
    }

    fn adjust_equalization(&mut self, _params: EqParams) -> Result<(), Infallible> {
        Ok(())
    }

    fn set_scrambling(&mut self, enabled: bool) -> Result<(), Infallible> {
        println!("PHY:  set_scrambling({enabled})");
        Ok(())
    }
}

// --- main ------------------------------------------------------------------------

fn main() {
    let rate = HdmiForumFrl::Rate6Gbps4Lanes;
    let config = TrainingConfig::default();

    println!("FRL training simulation — rate {rate:?}");
    println!();

    let mut trainer = FrlTrainer::new(SimScdc::new(), SimPhy);
    let (outcome, trace) = trainer
        .train_at_rate_traced(rate, &config)
        .expect("SimScdc and SimPhy are infallible");

    println!();
    println!("Outcome: {outcome:?}");
    println!();
    println!("Trace ({} events):", trace.events.len());
    for event in &trace.events {
        println!("  {event:?}");
    }
}
