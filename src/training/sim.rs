extern crate std;
use std::collections::VecDeque;

use display_types::cea861::hdmi_forum::HdmiForumFrl;
use hdmi_hal::phy::{EqParams, HdmiPhy, LtpPattern};

use crate::scdc::ScdcClient;
use crate::types::{CedCounters, FrlConfig, LtpReq, TrainingStatus};

use super::TrainingConfig;

// -------------------------------------------------------------------------
// SimScdc — scripted SCDC client for state machine tests
// -------------------------------------------------------------------------

pub struct SimScdc {
    /// Scripted responses for `read_training_status`, consumed in order.
    pub statuses: VecDeque<Result<TrainingStatus, ()>>,
    pub fail_write_frl_config: bool,
    pub fail_read_ced: bool,
    /// The last config written via `write_frl_config`.
    pub written_config: Option<FrlConfig>,
    pub ced_calls: u32,
}

impl SimScdc {
    pub fn new() -> Self {
        Self {
            statuses: VecDeque::new(),
            fail_write_frl_config: false,
            fail_read_ced: false,
            written_config: None,
            ced_calls: 0,
        }
    }

    pub fn push(&mut self, status: TrainingStatus) {
        self.statuses.push_back(Ok(status));
    }

    pub fn push_err(&mut self) {
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

pub struct MockPhy {
    pub fail_set_frl_rate: bool,
    pub fail_send_ltp: bool,
    pub frl_rate: Option<HdmiForumFrl>,
    pub last_ltp: Option<LtpPattern>,
}

impl MockPhy {
    pub fn new() -> Self {
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

pub fn not_ready() -> TrainingStatus {
    TrainingStatus {
        flt_ready: false,
        frl_start: false,
        ltp_req: LtpReq::None,
    }
}

pub fn flt_ready() -> TrainingStatus {
    TrainingStatus {
        flt_ready: true,
        frl_start: false,
        ltp_req: LtpReq::None,
    }
}

pub fn frl_started() -> TrainingStatus {
    TrainingStatus {
        flt_ready: true,
        frl_start: true,
        ltp_req: LtpReq::None,
    }
}

pub fn ltp(pattern: LtpReq) -> TrainingStatus {
    TrainingStatus {
        flt_ready: true,
        frl_start: true,
        ltp_req: pattern,
    }
}

pub const RATE: HdmiForumFrl = HdmiForumFrl::Rate6Gbps4Lanes;

pub fn cfg(flt: u32, frl: u32, ltp_t: u32) -> TrainingConfig {
    TrainingConfig {
        flt_ready_timeout: flt,
        frl_start_timeout: frl,
        ltp_timeout: ltp_t,
        ..TrainingConfig::default()
    }
}
