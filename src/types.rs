use display_types::cea861::hdmi_forum::HdmiForumFrl;

/// Link Training Pattern requested by the sink via Status_Flags_1 bits[7:4].
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LtpReq {
    /// No pattern requested; all lanes satisfied.
    None = 0,
    /// LFSR pattern 0.
    Lfsr0 = 1,
    /// LFSR pattern 1.
    Lfsr1 = 2,
    /// LFSR pattern 2.
    Lfsr2 = 3,
    /// LFSR pattern 3.
    Lfsr3 = 4,
}

/// FFE (Feed-Forward Equalization) level count advertised to the sink in Config_0.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FfeLevels {
    /// 0 FFE levels.
    Ffe0 = 0,
    /// 1 FFE level.
    Ffe1 = 1,
    /// 2 FFE levels.
    Ffe2 = 2,
    /// 3 FFE levels.
    Ffe3 = 3,
    /// 4 FFE levels.
    Ffe4 = 4,
    /// 5 FFE levels.
    Ffe5 = 5,
    /// 6 FFE levels.
    Ffe6 = 6,
    /// 7 FFE levels.
    Ffe7 = 7,
}

/// FRL configuration written to Config_0.
///
/// `dsc_frl_max` reflects whether the negotiated configuration requires DSC transport.
/// plumbob passes it through without interpreting it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FrlConfig {
    /// The target FRL rate.
    pub rate: HdmiForumFrl,
    /// FFE level count advertised to the sink.
    pub ffe_levels: FfeLevels,
    /// Whether to set DSC_FRL_Max in Config_0.
    pub dsc_frl_max: bool,
}

/// The subset of SCDC status that the training state machine reads on each poll.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TrainingStatus {
    /// The sink has completed internal preparation for FRL training at this rate.
    pub flt_ready: bool,
    /// The sink is ready for the LTP training loop to begin.
    pub frl_start: bool,
    /// The LTP pattern currently requested by the sink.
    pub ltp_req: LtpReq,
}

/// A 15-bit per-lane character error count.
///
/// The high byte's bit 7 is a validity flag consumed by [`CedCounters`];
/// the counter occupies bits[14:0]. Values are always ≤ `0x7FFF`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CedCount(u16);

impl CedCount {
    /// Constructs a `CedCount`, masking to 15 bits.
    pub fn new(raw: u16) -> Self {
        Self(raw & 0x7FFF)
    }

    /// Returns the character error count.
    pub fn value(self) -> u16 {
        self.0
    }
}

/// Per-lane character error counts used for equalization feedback.
///
/// A lane's counter is `None` when its validity bit is not set. `lane3` is only
/// populated in 4-lane FRL mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CedCounters {
    /// Character error count for lane 0, or `None` if the validity bit is not set.
    pub lane0: Option<CedCount>,
    /// Character error count for lane 1, or `None` if the validity bit is not set.
    pub lane1: Option<CedCount>,
    /// Character error count for lane 2, or `None` if the validity bit is not set.
    pub lane2: Option<CedCount>,
    /// Character error count for lane 3, or `None` if the validity bit is not set.
    /// Always `None` in 3-lane FRL mode.
    pub lane3: Option<CedCount>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use display_types::cea861::hdmi_forum::HdmiForumFrl;

    // --- LtpReq ---

    #[test]
    fn ltp_req_discriminants() {
        assert_eq!(LtpReq::None as u8, 0);
        assert_eq!(LtpReq::Lfsr0 as u8, 1);
        assert_eq!(LtpReq::Lfsr1 as u8, 2);
        assert_eq!(LtpReq::Lfsr2 as u8, 3);
        assert_eq!(LtpReq::Lfsr3 as u8, 4);
    }

    #[test]
    fn ltp_req_clone_eq() {
        let a = LtpReq::Lfsr2;
        assert_eq!(a, a.clone());
        assert_ne!(LtpReq::Lfsr0, LtpReq::Lfsr1);
    }

    // --- FfeLevels ---

    #[test]
    fn ffe_levels_discriminants() {
        assert_eq!(FfeLevels::Ffe0 as u8, 0);
        assert_eq!(FfeLevels::Ffe7 as u8, 7);
    }

    #[test]
    fn ffe_levels_clone_eq() {
        let a = FfeLevels::Ffe3;
        assert_eq!(a, a.clone());
        assert_ne!(FfeLevels::Ffe0, FfeLevels::Ffe7);
    }

    // --- FrlConfig ---

    #[test]
    fn frl_config_fields() {
        let cfg = FrlConfig {
            rate: HdmiForumFrl::Rate6Gbps4Lanes,
            ffe_levels: FfeLevels::Ffe2,
            dsc_frl_max: true,
        };
        assert_eq!(cfg.rate, HdmiForumFrl::Rate6Gbps4Lanes);
        assert_eq!(cfg.ffe_levels, FfeLevels::Ffe2);
        assert!(cfg.dsc_frl_max);
    }

    #[test]
    fn frl_config_clone_eq() {
        let a = FrlConfig {
            rate: HdmiForumFrl::Rate3Gbps3Lanes,
            ffe_levels: FfeLevels::Ffe0,
            dsc_frl_max: false,
        };
        assert_eq!(a, a.clone());
    }

    // --- TrainingStatus ---

    #[test]
    fn training_status_fields() {
        let s = TrainingStatus { flt_ready: true, frl_start: false, ltp_req: LtpReq::Lfsr1 };
        assert!(s.flt_ready);
        assert!(!s.frl_start);
        assert_eq!(s.ltp_req, LtpReq::Lfsr1);
    }

    #[test]
    fn training_status_clone_eq() {
        let a = TrainingStatus { flt_ready: true, frl_start: true, ltp_req: LtpReq::None };
        assert_eq!(a, a.clone());
    }

    // --- CedCount ---

    #[test]
    fn ced_count_masks_validity_bit() {
        // Bit 15 is the validity flag; it must be stripped.
        let c = CedCount::new(0xFFFF);
        assert_eq!(c.value(), 0x7FFF);
    }

    #[test]
    fn ced_count_preserves_15_bit_value() {
        let c = CedCount::new(0x0123);
        assert_eq!(c.value(), 0x0123);
    }

    #[test]
    fn ced_count_zero() {
        assert_eq!(CedCount::new(0).value(), 0);
    }

    #[test]
    fn ced_count_clone_eq() {
        let a = CedCount::new(42);
        assert_eq!(a, a.clone());
        assert_ne!(CedCount::new(1), CedCount::new(2));
    }

    // --- CedCounters ---

    #[test]
    fn ced_counters_all_none() {
        let c = CedCounters { lane0: None, lane1: None, lane2: None, lane3: None };
        assert!(c.lane0.is_none());
        assert!(c.lane3.is_none());
    }

    #[test]
    fn ced_counters_individual_lanes() {
        let c = CedCounters {
            lane0: Some(CedCount::new(10)),
            lane1: Some(CedCount::new(20)),
            lane2: None,
            lane3: Some(CedCount::new(30)),
        };
        assert_eq!(c.lane0.unwrap().value(), 10);
        assert_eq!(c.lane1.unwrap().value(), 20);
        assert!(c.lane2.is_none());
        assert_eq!(c.lane3.unwrap().value(), 30);
    }

    #[test]
    fn ced_counters_clone_eq() {
        let a = CedCounters { lane0: Some(CedCount::new(5)), lane1: None, lane2: None, lane3: None };
        assert_eq!(a, a.clone());
    }
}
