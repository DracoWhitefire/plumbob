use display_types::cea861::hdmi_forum::HdmiForumFrl;
use hdmi_hal::phy::LtpPattern;

/// Link training pattern requested by the sink for one lane: a 4-bit field in
/// `Status_Flags_1` (lanes 0–1) or `Status_Flags_2` (lanes 2–3).
///
/// Values 0x1–0x8 are patterns the PHY drives on that lane; 0x0, 0xE and 0xF are
/// signals to the state machine and never reach the PHY. The values the specification
/// leaves undefined (0x9–0xD) are [`Reserved`](Self::Reserved), so a request can always
/// be represented and the training layer decides what to do with it: plumbob leaves the
/// lane as it was.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LtpReq {
    /// No pattern requested (0x0): the lane is trained.
    None,
    /// All ones (0x1).
    AllOnes,
    /// All zeros (0x2).
    AllZeros,
    /// Nyquist clock pattern (0x3).
    NyquistClock,
    /// DDE (Data Dependent Equalization) compliance pattern (0x4).
    DdeCompliance,
    /// LFSR pattern 0 (0x5).
    Lfsr0,
    /// LFSR pattern 1 (0x6).
    Lfsr1,
    /// LFSR pattern 2 (0x7).
    Lfsr2,
    /// LFSR pattern 3 (0x8).
    Lfsr3,
    /// Raise this lane's TxFFE level (0xE).
    FfeChange,
    /// Drop the FRL rate (0xF).
    RateChange,
    /// A value the specification leaves undefined, 0x9–0xD.
    Reserved(u8),
}

impl LtpReq {
    /// The request's 4-bit value.
    pub const fn value(self) -> u8 {
        match self {
            Self::None => 0x0,
            Self::AllOnes => 0x1,
            Self::AllZeros => 0x2,
            Self::NyquistClock => 0x3,
            Self::DdeCompliance => 0x4,
            Self::Lfsr0 => 0x5,
            Self::Lfsr1 => 0x6,
            Self::Lfsr2 => 0x7,
            Self::Lfsr3 => 0x8,
            Self::FfeChange => 0xE,
            Self::RateChange => 0xF,
            Self::Reserved(value) => value,
        }
    }

    /// The pattern this request asks the PHY to drive, or `None` for the requests
    /// that are signals to the state machine (0x0, 0xE and 0xF) and for reserved values.
    pub const fn pattern(self) -> Option<LtpPattern> {
        match self {
            Self::None | Self::FfeChange | Self::RateChange | Self::Reserved(_) => None,
            Self::AllOnes => Some(LtpPattern::AllOnes),
            Self::AllZeros => Some(LtpPattern::AllZeros),
            Self::NyquistClock => Some(LtpPattern::NyquistClock),
            Self::DdeCompliance => Some(LtpPattern::DdeCompliance),
            Self::Lfsr0 => Some(LtpPattern::Lfsr0),
            Self::Lfsr1 => Some(LtpPattern::Lfsr1),
            Self::Lfsr2 => Some(LtpPattern::Lfsr2),
            Self::Lfsr3 => Some(LtpPattern::Lfsr3),
        }
    }
}

/// The sink's requests for all four lanes. Lane 3 is ignored in 3-lane FRL.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LtpRequests {
    /// Request for lane 0.
    pub lane0: LtpReq,
    /// Request for lane 1.
    pub lane1: LtpReq,
    /// Request for lane 2.
    pub lane2: LtpReq,
    /// Request for lane 3. Ignored in 3-lane FRL.
    pub lane3: LtpReq,
}

/// The highest TxFFE level index the source supports, written to `Config_1` bits 7:4.
///
/// At most 3 at rates up to 12 Gbps per lane and at most 7 above; see
/// [`limited_to`](Self::limited_to). The default is 0.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord)]
pub struct FfeLevels(u8);

impl FfeLevels {
    /// The highest level the `Config_1` field and the PHY accept, 7.
    pub const MAX: Self = Self(7);

    /// Returns the level for `level`, or `None` if it is above 7.
    pub const fn new(level: u8) -> Option<Self> {
        if level <= Self::MAX.0 {
            Some(Self(level))
        } else {
            None
        }
    }

    /// Returns the level index (0–7).
    pub const fn value(self) -> u8 {
        self.0
    }

    /// Returns this level, capped at the maximum for `rate`: 3 at rates up to 12 Gbps
    /// per lane, 7 above.
    pub fn limited_to(self, rate: HdmiForumFrl) -> Self {
        // No rate above 12 Gbps exists in display-types yet; the 7 is for the faster
        // rates when they are added.
        let top_3_level_rate = HdmiForumFrl::Rate12Gbps4Lanes;
        let max = if rate <= top_3_level_rate { 3 } else { 7 };
        Self(self.0.min(max))
    }
}

/// Written to `Config_1`: the FRL rate (bits 3:0) and the FFE levels (bits 7:4).
///
/// `HdmiForumFrl::NotSupported` turns FRL off (LTS:L).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FrlConfig {
    /// The FRL rate.
    pub rate: HdmiForumFrl,
    /// The highest TxFFE level the source will use at this rate.
    pub ffe_levels: FfeLevels,
}

/// The `Update_0` flags the state machine reads and clears.
///
/// Other `Update_0` flags are not part of training and stay in the SCDC implementation.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct UpdateFlags {
    /// `Source_Test_Update`: the sink changed `Source_Test_Configuration`.
    pub source_test_update: bool,
    /// `FRL_start`: the sink is ready for the source to start FRL transmission.
    pub frl_start: bool,
    /// `FLT_update`: the sink has posted new link training requests.
    pub flt_update: bool,
}

/// The `Source_Test_Configuration` field the state machine honours.
///
/// Other fields are not part of training and stay in the SCDC implementation.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SourceTestConfig {
    /// `FLT_no_timeout`: suspend the LTS:2 and LTS:3 poll limits (compliance testing).
    pub flt_no_timeout: bool,
}

/// A 15-bit per-lane character error count.
///
/// The high byte's bit 7 is a validity flag consumed by [`CedCounters`];
/// the counter occupies `bits[14:0]`. Values are always ≤ `0x7FFF`.
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

/// Per-lane character error counts, for diagnostics; the training procedure does not
/// consume them.
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

    // --- LtpReq ---

    #[test]
    fn ltp_req_values_match_status_flags_encoding() {
        assert_eq!(LtpReq::None.value(), 0x0);
        assert_eq!(LtpReq::AllOnes.value(), 0x1);
        assert_eq!(LtpReq::AllZeros.value(), 0x2);
        assert_eq!(LtpReq::NyquistClock.value(), 0x3);
        assert_eq!(LtpReq::DdeCompliance.value(), 0x4);
        assert_eq!(LtpReq::Lfsr0.value(), 0x5);
        assert_eq!(LtpReq::Lfsr1.value(), 0x6);
        assert_eq!(LtpReq::Lfsr2.value(), 0x7);
        assert_eq!(LtpReq::Lfsr3.value(), 0x8);
        assert_eq!(LtpReq::FfeChange.value(), 0xE);
        assert_eq!(LtpReq::RateChange.value(), 0xF);
        assert_eq!(LtpReq::Reserved(0xB).value(), 0xB);
    }

    #[test]
    fn ltp_req_patterns_match_by_value() {
        assert_eq!(LtpReq::AllOnes.pattern(), Some(LtpPattern::AllOnes));
        assert_eq!(LtpReq::AllZeros.pattern(), Some(LtpPattern::AllZeros));
        assert_eq!(
            LtpReq::NyquistClock.pattern(),
            Some(LtpPattern::NyquistClock)
        );
        assert_eq!(
            LtpReq::DdeCompliance.pattern(),
            Some(LtpPattern::DdeCompliance)
        );
        assert_eq!(LtpReq::Lfsr0.pattern(), Some(LtpPattern::Lfsr0));
        assert_eq!(LtpReq::Lfsr1.pattern(), Some(LtpPattern::Lfsr1));
        assert_eq!(LtpReq::Lfsr2.pattern(), Some(LtpPattern::Lfsr2));
        assert_eq!(LtpReq::Lfsr3.pattern(), Some(LtpPattern::Lfsr3));
    }

    #[test]
    fn ltp_req_patterns_share_their_value_with_ltp_pattern() {
        for req in [
            LtpReq::AllOnes,
            LtpReq::AllZeros,
            LtpReq::NyquistClock,
            LtpReq::DdeCompliance,
            LtpReq::Lfsr0,
            LtpReq::Lfsr1,
            LtpReq::Lfsr2,
            LtpReq::Lfsr3,
        ] {
            assert_eq!(req.pattern().map(LtpPattern::value), Some(req.value()));
        }
    }

    #[test]
    fn ltp_req_signals_have_no_pattern() {
        assert_eq!(LtpReq::None.pattern(), None);
        assert_eq!(LtpReq::FfeChange.pattern(), None);
        assert_eq!(LtpReq::RateChange.pattern(), None);
        assert_eq!(LtpReq::Reserved(0x9).pattern(), None);
    }

    // --- FfeLevels ---

    #[test]
    fn ffe_levels_accepts_0_to_7() {
        for level in 0..=7 {
            assert_eq!(FfeLevels::new(level).map(FfeLevels::value), Some(level));
        }
    }

    #[test]
    fn ffe_levels_rejects_above_7() {
        assert_eq!(FfeLevels::new(8), None);
        assert_eq!(FfeLevels::new(u8::MAX), None);
    }

    #[test]
    fn ffe_levels_max_and_default() {
        assert_eq!(FfeLevels::MAX.value(), 7);
        assert_eq!(FfeLevels::default().value(), 0);
    }

    #[test]
    fn ffe_levels_limited_to_3_up_to_12_gbps() {
        for rate in [
            HdmiForumFrl::Rate3Gbps3Lanes,
            HdmiForumFrl::Rate6Gbps3Lanes,
            HdmiForumFrl::Rate6Gbps4Lanes,
            HdmiForumFrl::Rate8Gbps4Lanes,
            HdmiForumFrl::Rate10Gbps4Lanes,
            HdmiForumFrl::Rate12Gbps4Lanes,
        ] {
            assert_eq!(FfeLevels::MAX.limited_to(rate).value(), 3);
        }
    }

    #[test]
    fn ffe_levels_below_the_limit_are_unchanged() {
        let two = FfeLevels::new(2).unwrap();
        assert_eq!(two.limited_to(HdmiForumFrl::Rate12Gbps4Lanes), two);
        assert_eq!(
            FfeLevels::default().limited_to(HdmiForumFrl::Rate3Gbps3Lanes),
            FfeLevels::default()
        );
    }

    // --- FrlConfig, UpdateFlags, SourceTestConfig ---

    #[test]
    fn update_flags_default_is_all_clear() {
        let flags = UpdateFlags::default();
        assert!(!flags.source_test_update);
        assert!(!flags.frl_start);
        assert!(!flags.flt_update);
    }

    #[test]
    fn source_test_config_default_keeps_timeouts() {
        assert!(!SourceTestConfig::default().flt_no_timeout);
    }

    #[test]
    fn ltp_requests_compare_per_lane() {
        let a = LtpRequests {
            lane0: LtpReq::Lfsr0,
            lane1: LtpReq::Lfsr1,
            lane2: LtpReq::Lfsr2,
            lane3: LtpReq::Lfsr3,
        };
        assert_eq!(a, a);
        assert_ne!(
            a,
            LtpRequests {
                lane3: LtpReq::None,
                ..a
            }
        );
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
}
