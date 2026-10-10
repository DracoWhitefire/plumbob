//! Simulated FRL training example for `plumbob`.
//!
//! Demonstrates the full `FrlTrainer` usage pattern against in-memory
//! implementations of `ScdcClient` and `HdmiPhy`. No hardware required.
//!
//! Run with `cargo run` from this directory.

use core::convert::Infallible;
use display_types::cea861::hdmi_forum::HdmiForumFrl;
use hdmi_hal::phy::{EqParams, FrlOutput, HdmiPhy, LaneEqParams, LanePatterns};
use plumbob::{
    CedCounters, FfeLevels, FrlConfig, FrlTrainer, LtpReq, LtpRequests, ScdcClient,
    SourceTestConfig, TrainingConfig, UpdateFlags,
};

// --- SimSink ---------------------------------------------------------------------
//
// A sink that asks for a lower rate and then trains at it:
//
//   LTS:2  — FLT_ready asserts on the third poll.
//   LTS:3  — at the first rate the sink requests a lower rate (0xF on every lane);
//            at the next rate it requests one LFSR pattern per lane, then a TxFFE
//            raise on lane 1, then passes every lane.
//   LTS:P  — FRL_start asserts on the third poll.
//
// Each set of requests is posted with FLT_update as soon as the previous one is
// cleared, once Config_1 holds an FRL rate.

const fn lanes(lane0: LtpReq, lane1: LtpReq, lane2: LtpReq, lane3: LtpReq) -> LtpRequests {
    LtpRequests {
        lane0,
        lane1,
        lane2,
        lane3,
    }
}

const REQUESTS: [LtpRequests; 4] = [
    lanes(
        LtpReq::RateChange,
        LtpReq::RateChange,
        LtpReq::RateChange,
        LtpReq::RateChange,
    ),
    lanes(LtpReq::Lfsr0, LtpReq::Lfsr1, LtpReq::Lfsr2, LtpReq::Lfsr3),
    lanes(LtpReq::None, LtpReq::FfeChange, LtpReq::None, LtpReq::None),
    lanes(LtpReq::None, LtpReq::None, LtpReq::None, LtpReq::None),
];

#[derive(Default)]
struct SimSink {
    flt_ready_polls: u32,
    configured: bool,
    next_request: usize,
    flt_update: bool,
    frl_start_polls: u32,
    frl_start: bool,
}

impl ScdcClient for SimSink {
    type Error = Infallible;

    fn read_flt_ready(&mut self) -> Result<bool, Infallible> {
        self.flt_ready_polls += 1;
        Ok(self.flt_ready_polls >= 3)
    }

    fn read_update_flags(&mut self) -> Result<UpdateFlags, Infallible> {
        if self.configured && !self.flt_update {
            if self.next_request < REQUESTS.len() {
                self.flt_update = true;
            } else if !self.frl_start {
                self.frl_start_polls += 1;
                self.frl_start = self.frl_start_polls >= 3;
            }
        }
        Ok(UpdateFlags {
            source_test_update: false,
            frl_start: self.frl_start,
            flt_update: self.flt_update,
        })
    }

    fn clear_update_flags(&mut self, flags: UpdateFlags) -> Result<(), Infallible> {
        if flags.flt_update && self.flt_update {
            self.flt_update = false;
            self.next_request += 1;
        }
        if flags.frl_start {
            println!("SCDC: clear FRL_start");
            self.frl_start = false;
        }
        Ok(())
    }

    fn read_ltp_requests(&mut self) -> Result<LtpRequests, Infallible> {
        let requests = REQUESTS[self.next_request];
        println!("SCDC: requests {requests:?}");
        Ok(requests)
    }

    fn read_source_test_config(&mut self) -> Result<SourceTestConfig, Infallible> {
        Ok(SourceTestConfig::default())
    }

    fn write_config_0_defaults(&mut self) -> Result<(), Infallible> {
        println!("SCDC: write Config_0 defaults");
        Ok(())
    }

    fn write_frl_config(&mut self, config: FrlConfig) -> Result<(), Infallible> {
        println!(
            "SCDC: write Config_1 (rate={:?}, ffe_levels={})",
            config.rate,
            config.ffe_levels.value()
        );
        self.configured = config.rate != HdmiForumFrl::NotSupported;
        Ok(())
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

    fn send_ltp(&mut self, patterns: LanePatterns) -> Result<(), Infallible> {
        println!("PHY:  send_ltp({patterns:?})");
        Ok(())
    }

    fn set_frl_output(&mut self, output: FrlOutput) -> Result<(), Infallible> {
        println!("PHY:  set_frl_output({output:?})");
        Ok(())
    }

    fn adjust_equalization(&mut self, params: EqParams) -> Result<(), Infallible> {
        let level = |lane: LaneEqParams| lane.tx_ffe_level.value();
        println!(
            "PHY:  adjust_equalization(TxFFE {} {} {} {:?})",
            level(params.lane0),
            level(params.lane1),
            level(params.lane2),
            params.lane3.map(level)
        );
        Ok(())
    }

    fn set_scrambling(&mut self, enabled: bool) -> Result<(), Infallible> {
        println!("PHY:  set_scrambling({enabled})");
        Ok(())
    }
}

// --- main ------------------------------------------------------------------------

fn main() {
    let rates = [
        HdmiForumFrl::Rate12Gbps4Lanes,
        HdmiForumFrl::Rate10Gbps4Lanes,
    ];
    let mut config = TrainingConfig::default();
    config.ffe_levels = FfeLevels::new(3).expect("3 is a valid FFE level");

    println!("FRL training simulation — rates {rates:?}");
    println!();

    let mut trainer = FrlTrainer::new(SimSink::default(), SimPhy);
    let (result, trace) = trainer.train_traced(&rates, &config);
    let outcome = result.expect("SimSink and SimPhy are infallible");

    println!();
    println!("Outcome: {outcome:?}");
    println!();
    println!("Trace ({} events):", trace.events.len());
    for event in &trace.events {
        println!("  {event:?}");
    }
}
