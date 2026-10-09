//! A simulated sink and PHY for exercising the state machine without hardware.
//!
//! [`SimSink`] answers the state machine's SCDC calls from a short script and records
//! every call it receives. [`SimPhy`] records every PHY call. Both can be told to fail a
//! given operation.

extern crate std;

use std::collections::VecDeque;
use std::vec::Vec;

use display_types::cea861::hdmi_forum::HdmiForumFrl;
use hdmi_hal::phy::{EqParams, FrlOutput, HdmiPhy, LanePatterns};

use crate::scdc::ScdcClient;
use crate::types::{CedCounters, FrlConfig, LtpReq, LtpRequests, SourceTestConfig, UpdateFlags};

/// The same request on all four lanes.
pub fn all(req: LtpReq) -> LtpRequests {
    LtpRequests {
        lane0: req,
        lane1: req,
        lane2: req,
        lane3: req,
    }
}

/// One set of LTS:3 requests: after `after_polls` polls of `Update_0` that see nothing,
/// the sink sets `FLT_update` and posts `requests` (and, with `source_test`, changes its
/// source test configuration at the same time). The round ends when the source clears
/// `FLT_update`.
#[derive(Debug, Clone, Copy)]
pub struct Round {
    pub after_polls: u32,
    pub requests: LtpRequests,
    pub source_test: Option<SourceTestConfig>,
}

/// An SCDC operation, for failure injection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SinkOp {
    ReadFltReady,
    ReadUpdateFlags,
    ClearUpdateFlags,
    ReadLtpRequests,
    ReadSourceTestConfig,
    WriteConfig0Defaults,
    WriteFrlConfig,
    ReadCed,
}

/// A call the sink received, with what it returned or was given.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SinkCall {
    ReadFltReady(bool),
    ReadUpdateFlags(UpdateFlags),
    ClearUpdateFlags(UpdateFlags),
    ReadLtpRequests(LtpRequests),
    ReadSourceTestConfig(SourceTestConfig),
    WriteConfig0Defaults,
    WriteFrlConfig(FrlConfig),
    ReadCed,
}

/// A scripted sink.
///
/// - `FLT_ready` reads false for the first `flt_ready_after` polls, then true; never,
///   if not set.
/// - Each [`Round`] raises `FLT_update` in turn, counting polls from the last write of
///   `Config_1` with an FRL rate (a sink starts training once it is configured). A rate
///   drop is a round of `RateChange`; a retrain during LTS:P is a round after the
///   all-`None` one.
/// - Once the rounds are used up, `FRL_start` is set after `frl_start_after` more polls
///   of `Update_0`; never, if not set.
/// - With a source test configuration, `Source_Test_Update` is set until cleared.
#[derive(Debug, Default)]
pub struct SimSink {
    flt_ready_after: Option<u32>,
    rounds: VecDeque<Round>,
    frl_start_after: Option<u32>,
    source_test: SourceTestConfig,
    fail: Option<SinkOp>,

    flt_ready_polls: u32,
    configured: bool,
    update_polls: u32,
    flags: UpdateFlags,

    /// Every call received, in order. Failed calls are not recorded.
    pub calls: Vec<SinkCall>,
}

impl SimSink {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn flt_ready_after(mut self, polls: u32) -> Self {
        self.flt_ready_after = Some(polls);
        self
    }

    pub fn round(mut self, after_polls: u32, requests: LtpRequests) -> Self {
        self.rounds.push_back(Round {
            after_polls,
            requests,
            source_test: None,
        });
        self
    }

    pub fn round_with_source_test(
        mut self,
        after_polls: u32,
        requests: LtpRequests,
        source_test: SourceTestConfig,
    ) -> Self {
        self.rounds.push_back(Round {
            after_polls,
            requests,
            source_test: Some(source_test),
        });
        self
    }

    pub fn frl_start_after(mut self, polls: u32) -> Self {
        self.frl_start_after = Some(polls);
        self
    }

    pub fn source_test(mut self, config: SourceTestConfig) -> Self {
        self.source_test = config;
        self.flags.source_test_update = true;
        self
    }

    pub fn fail(mut self, op: SinkOp) -> Self {
        self.fail = Some(op);
        self
    }

    fn check(&self, op: SinkOp) -> Result<(), ()> {
        if self.fail == Some(op) {
            Err(())
        } else {
            Ok(())
        }
    }
}

impl ScdcClient for SimSink {
    type Error = ();

    fn read_flt_ready(&mut self) -> Result<bool, ()> {
        self.check(SinkOp::ReadFltReady)?;
        let ready = self
            .flt_ready_after
            .is_some_and(|n| self.flt_ready_polls >= n);
        self.flt_ready_polls += 1;
        self.calls.push(SinkCall::ReadFltReady(ready));
        Ok(ready)
    }

    fn read_update_flags(&mut self) -> Result<UpdateFlags, ()> {
        self.check(SinkOp::ReadUpdateFlags)?;
        if self.configured {
            match self.rounds.front().copied() {
                Some(round) if !self.flags.flt_update && self.update_polls >= round.after_polls => {
                    self.flags.flt_update = true;
                    if let Some(config) = round.source_test {
                        self.source_test = config;
                        self.flags.source_test_update = true;
                    }
                }
                Some(_) => {}
                None => {
                    self.flags.frl_start |=
                        self.frl_start_after.is_some_and(|n| self.update_polls >= n)
                }
            }
            self.update_polls += 1;
        }
        self.calls.push(SinkCall::ReadUpdateFlags(self.flags));
        Ok(self.flags)
    }

    fn clear_update_flags(&mut self, flags: UpdateFlags) -> Result<(), ()> {
        self.check(SinkOp::ClearUpdateFlags)?;
        if flags.flt_update && self.flags.flt_update {
            self.flags.flt_update = false;
            self.rounds.pop_front();
            self.update_polls = 0;
        }
        if flags.frl_start {
            // A sink sets FRL_start once; it does not come back after being cleared.
            self.flags.frl_start = false;
            self.frl_start_after = None;
        }
        self.flags.source_test_update &= !flags.source_test_update;
        self.calls.push(SinkCall::ClearUpdateFlags(flags));
        Ok(())
    }

    fn read_ltp_requests(&mut self) -> Result<LtpRequests, ()> {
        self.check(SinkOp::ReadLtpRequests)?;
        let requests = self
            .rounds
            .front()
            .map_or(all(LtpReq::None), |round| round.requests);
        self.calls.push(SinkCall::ReadLtpRequests(requests));
        Ok(requests)
    }

    fn read_source_test_config(&mut self) -> Result<SourceTestConfig, ()> {
        self.check(SinkOp::ReadSourceTestConfig)?;
        self.calls
            .push(SinkCall::ReadSourceTestConfig(self.source_test));
        Ok(self.source_test)
    }

    fn write_config_0_defaults(&mut self) -> Result<(), ()> {
        self.check(SinkOp::WriteConfig0Defaults)?;
        self.calls.push(SinkCall::WriteConfig0Defaults);
        Ok(())
    }

    fn write_frl_config(&mut self, config: FrlConfig) -> Result<(), ()> {
        self.check(SinkOp::WriteFrlConfig)?;
        self.configured = config.rate != HdmiForumFrl::NotSupported;
        self.update_polls = 0;
        self.calls.push(SinkCall::WriteFrlConfig(config));
        Ok(())
    }

    fn read_ced(&mut self) -> Result<CedCounters, ()> {
        self.check(SinkOp::ReadCed)?;
        self.calls.push(SinkCall::ReadCed);
        Ok(CedCounters {
            lane0: None,
            lane1: None,
            lane2: None,
            lane3: None,
        })
    }
}

/// A PHY operation, for failure injection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PhyOp {
    SetFrlRate,
    SendLtp,
    SetFrlOutput,
    AdjustEqualization,
    SetScrambling,
}

/// A call the PHY received.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PhyCall {
    SetFrlRate(HdmiForumFrl),
    SendLtp(LanePatterns),
    SetFrlOutput(FrlOutput),
    AdjustEqualization(EqParams),
    SetScrambling(bool),
}

/// A PHY that records every call.
#[derive(Debug, Default)]
pub struct SimPhy {
    fail: Option<PhyOp>,
    /// Every call received, in order. Failed calls are not recorded.
    pub calls: Vec<PhyCall>,
}

impl SimPhy {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn fail(mut self, op: PhyOp) -> Self {
        self.fail = Some(op);
        self
    }

    fn record(&mut self, op: PhyOp, call: PhyCall) -> Result<(), ()> {
        if self.fail == Some(op) {
            return Err(());
        }
        self.calls.push(call);
        Ok(())
    }
}

impl HdmiPhy for SimPhy {
    type Error = ();

    fn set_frl_rate(&mut self, rate: HdmiForumFrl) -> Result<(), ()> {
        self.record(PhyOp::SetFrlRate, PhyCall::SetFrlRate(rate))
    }

    fn send_ltp(&mut self, patterns: LanePatterns) -> Result<(), ()> {
        self.record(PhyOp::SendLtp, PhyCall::SendLtp(patterns))
    }

    fn set_frl_output(&mut self, output: FrlOutput) -> Result<(), ()> {
        self.record(PhyOp::SetFrlOutput, PhyCall::SetFrlOutput(output))
    }

    fn adjust_equalization(&mut self, params: EqParams) -> Result<(), ()> {
        self.record(
            PhyOp::AdjustEqualization,
            PhyCall::AdjustEqualization(params),
        )
    }

    fn set_scrambling(&mut self, enabled: bool) -> Result<(), ()> {
        self.record(PhyOp::SetScrambling, PhyCall::SetScrambling(enabled))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::FfeLevels;

    const FLT_UPDATE: UpdateFlags = UpdateFlags {
        source_test_update: false,
        frl_start: false,
        flt_update: true,
    };

    /// Writes `Config_1` with an FRL rate, which starts the sink's rounds.
    fn configured(mut sink: SimSink) -> SimSink {
        sink.write_frl_config(FrlConfig {
            rate: HdmiForumFrl::Rate6Gbps4Lanes,
            ffe_levels: FfeLevels::default(),
        })
        .unwrap();
        sink
    }

    // --- SimSink: FLT_ready

    #[test]
    fn flt_ready_is_false_until_after_the_given_polls() {
        let mut sink = SimSink::new().flt_ready_after(2);
        assert_eq!(sink.read_flt_ready(), Ok(false));
        assert_eq!(sink.read_flt_ready(), Ok(false));
        assert_eq!(sink.read_flt_ready(), Ok(true));
        assert_eq!(sink.read_flt_ready(), Ok(true));
    }

    #[test]
    fn flt_ready_never_asserts_unless_scripted() {
        let mut sink = SimSink::new();
        for _ in 0..10 {
            assert_eq!(sink.read_flt_ready(), Ok(false));
        }
    }

    // --- SimSink: rounds and FLT_update

    #[test]
    fn round_raises_flt_update_after_its_polls() {
        let mut sink = configured(SimSink::new().round(1, all(LtpReq::Lfsr0)));
        assert!(!sink.read_update_flags().unwrap().flt_update);
        assert!(sink.read_update_flags().unwrap().flt_update);
        assert_eq!(sink.read_ltp_requests(), Ok(all(LtpReq::Lfsr0)));
    }

    #[test]
    fn clearing_flt_update_moves_to_the_next_round() {
        let mut sink = configured(
            SimSink::new()
                .round(0, all(LtpReq::Lfsr0))
                .round(0, all(LtpReq::None)),
        );
        assert!(sink.read_update_flags().unwrap().flt_update);
        sink.clear_update_flags(FLT_UPDATE).unwrap();
        assert!(sink.read_update_flags().unwrap().flt_update);
        assert_eq!(sink.read_ltp_requests(), Ok(all(LtpReq::None)));
    }

    #[test]
    fn clearing_flt_update_when_not_set_keeps_the_round() {
        let mut sink = configured(SimSink::new().round(1, all(LtpReq::Lfsr1)));
        sink.clear_update_flags(FLT_UPDATE).unwrap();
        assert!(!sink.read_update_flags().unwrap().flt_update);
        assert!(sink.read_update_flags().unwrap().flt_update);
        assert_eq!(sink.read_ltp_requests(), Ok(all(LtpReq::Lfsr1)));
    }

    #[test]
    fn requests_read_as_none_once_rounds_are_used_up() {
        let mut sink = SimSink::new();
        assert_eq!(sink.read_ltp_requests(), Ok(all(LtpReq::None)));
    }

    // --- SimSink: FRL_start

    #[test]
    fn frl_start_follows_the_last_round() {
        let mut sink = configured(
            SimSink::new()
                .round(0, all(LtpReq::None))
                .frl_start_after(1),
        );
        assert!(sink.read_update_flags().unwrap().flt_update);
        sink.clear_update_flags(FLT_UPDATE).unwrap();
        assert!(!sink.read_update_flags().unwrap().frl_start);
        assert!(sink.read_update_flags().unwrap().frl_start);
        sink.clear_update_flags(UpdateFlags {
            frl_start: true,
            ..UpdateFlags::default()
        })
        .unwrap();
        assert!(!sink.read_update_flags().unwrap().frl_start);
    }

    #[test]
    fn rounds_wait_for_config_1_with_an_frl_rate() {
        let mut sink = SimSink::new()
            .round(0, all(LtpReq::Lfsr0))
            .frl_start_after(0);
        assert!(!sink.read_update_flags().unwrap().flt_update);
        sink.write_frl_config(FrlConfig {
            rate: HdmiForumFrl::NotSupported,
            ffe_levels: FfeLevels::default(),
        })
        .unwrap();
        assert!(!sink.read_update_flags().unwrap().flt_update);
        let mut sink = configured(sink);
        assert!(sink.read_update_flags().unwrap().flt_update);
    }

    #[test]
    fn round_with_source_test_changes_the_configuration() {
        let config = SourceTestConfig {
            flt_no_timeout: true,
        };
        let mut sink =
            configured(SimSink::new().round_with_source_test(0, all(LtpReq::NyquistClock), config));
        let flags = sink.read_update_flags().unwrap();
        assert!(flags.flt_update && flags.source_test_update);
        assert_eq!(sink.read_source_test_config(), Ok(config));
    }

    #[test]
    fn frl_start_never_asserts_unless_scripted() {
        let mut sink = configured(SimSink::new());
        for _ in 0..10 {
            assert!(!sink.read_update_flags().unwrap().frl_start);
        }
    }

    // --- SimSink: source test configuration

    #[test]
    fn source_test_sets_update_until_cleared() {
        let config = SourceTestConfig {
            flt_no_timeout: true,
        };
        let mut sink = SimSink::new().source_test(config);
        assert!(sink.read_update_flags().unwrap().source_test_update);
        assert_eq!(sink.read_source_test_config(), Ok(config));
        sink.clear_update_flags(UpdateFlags {
            source_test_update: true,
            ..UpdateFlags::default()
        })
        .unwrap();
        assert!(!sink.read_update_flags().unwrap().source_test_update);
    }

    #[test]
    fn source_test_config_defaults_to_timeouts_kept() {
        let mut sink = SimSink::new();
        assert_eq!(
            sink.read_source_test_config(),
            Ok(SourceTestConfig::default())
        );
    }

    // --- SimSink: recording and failures

    #[test]
    fn sink_records_calls_in_order() {
        let config = FrlConfig {
            rate: HdmiForumFrl::Rate6Gbps4Lanes,
            ffe_levels: FfeLevels::default(),
        };
        let mut sink = SimSink::new();
        sink.write_config_0_defaults().unwrap();
        sink.write_frl_config(config).unwrap();
        sink.read_ced().unwrap();
        assert_eq!(
            sink.calls,
            [
                SinkCall::WriteConfig0Defaults,
                SinkCall::WriteFrlConfig(config),
                SinkCall::ReadCed,
            ]
        );
    }

    #[test]
    fn sink_ced_reads_as_all_invalid() {
        let ced = SimSink::new().read_ced().unwrap();
        assert_eq!(ced.lane0, None);
        assert_eq!(ced.lane1, None);
        assert_eq!(ced.lane2, None);
        assert_eq!(ced.lane3, None);
    }

    #[test]
    fn sink_fails_the_chosen_operation_only() {
        let config = FrlConfig {
            rate: HdmiForumFrl::Rate6Gbps4Lanes,
            ffe_levels: FfeLevels::default(),
        };
        let ops = [
            SinkOp::ReadFltReady,
            SinkOp::ReadUpdateFlags,
            SinkOp::ClearUpdateFlags,
            SinkOp::ReadLtpRequests,
            SinkOp::ReadSourceTestConfig,
            SinkOp::WriteConfig0Defaults,
            SinkOp::WriteFrlConfig,
            SinkOp::ReadCed,
        ];
        for op in ops {
            let mut sink = SimSink::new().fail(op);
            let results = [
                sink.read_flt_ready().map(drop),
                sink.read_update_flags().map(drop),
                sink.clear_update_flags(UpdateFlags::default()),
                sink.read_ltp_requests().map(drop),
                sink.read_source_test_config().map(drop),
                sink.write_config_0_defaults(),
                sink.write_frl_config(config),
                sink.read_ced().map(drop),
            ];
            for (candidate, result) in ops.iter().zip(results) {
                assert_eq!(
                    result.is_err(),
                    *candidate == op,
                    "{candidate:?} with {op:?}"
                );
            }
            assert_eq!(sink.calls.len(), ops.len() - 1);
        }
    }

    // --- SimPhy

    #[test]
    fn phy_records_calls_in_order() {
        let mut phy = SimPhy::new();
        phy.set_frl_rate(HdmiForumFrl::Rate6Gbps4Lanes).unwrap();
        phy.send_ltp(LanePatterns::default()).unwrap();
        phy.set_frl_output(FrlOutput::GapOnly).unwrap();
        phy.adjust_equalization(EqParams::new()).unwrap();
        phy.set_scrambling(true).unwrap();
        assert_eq!(
            phy.calls,
            [
                PhyCall::SetFrlRate(HdmiForumFrl::Rate6Gbps4Lanes),
                PhyCall::SendLtp(LanePatterns::default()),
                PhyCall::SetFrlOutput(FrlOutput::GapOnly),
                PhyCall::AdjustEqualization(EqParams::new()),
                PhyCall::SetScrambling(true),
            ]
        );
    }

    #[test]
    fn phy_fails_the_chosen_operation_only() {
        let ops = [
            PhyOp::SetFrlRate,
            PhyOp::SendLtp,
            PhyOp::SetFrlOutput,
            PhyOp::AdjustEqualization,
            PhyOp::SetScrambling,
        ];
        for op in ops {
            let mut phy = SimPhy::new().fail(op);
            let results = [
                phy.set_frl_rate(HdmiForumFrl::Rate6Gbps4Lanes),
                phy.send_ltp(LanePatterns::default()),
                phy.set_frl_output(FrlOutput::Active),
                phy.adjust_equalization(EqParams::new()),
                phy.set_scrambling(false),
            ];
            for (candidate, result) in ops.iter().zip(results) {
                assert_eq!(
                    result.is_err(),
                    *candidate == op,
                    "{candidate:?} with {op:?}"
                );
            }
            assert_eq!(phy.calls.len(), ops.len() - 1);
        }
    }
}
