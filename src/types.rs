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
    ///
    /// Build requests from a raw value with [`from_value`](Self::from_value), which only
    /// produces `Reserved` for 0x9–0xD. A `Reserved` built by hand with any other value
    /// is not a request a sink can send: `Reserved(0x5)` has the value of `Lfsr0` but
    /// does not compare equal to it.
    Reserved(u8),
}

impl LtpReq {
    /// The request with the 4-bit `value`, or `None` above 0xF. Every value has exactly
    /// one request: 0x9–0xD are [`Reserved`](Self::Reserved).
    pub const fn from_value(value: u8) -> Option<Self> {
        Some(match value {
            0x0 => Self::None,
            0x1 => Self::AllOnes,
            0x2 => Self::AllZeros,
            0x3 => Self::NyquistClock,
            0x4 => Self::DdeCompliance,
            0x5 => Self::Lfsr0,
            0x6 => Self::Lfsr1,
            0x7 => Self::Lfsr2,
            0x8 => Self::Lfsr3,
            0x9..=0xD => Self::Reserved(value),
            0xE => Self::FfeChange,
            0xF => Self::RateChange,
            _ => return None,
        })
    }

    /// The request's 4-bit value (for a `Reserved` built by hand, the value it holds).
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

/// The sink's requests for all four lanes (`Status_Flags_1`, 0x41, and `Status_Flags_2`,
/// 0x42). Lane 3 is ignored in 3-lane FRL.
///
/// Mirrors culvert's `LtpRequests`. Build one with [`new`](Self::new).
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LtpRequests {
    /// Lane 0 (`Status_Flags_1` bits 3:0).
    pub lane0: LtpReq,
    /// Lane 1 (`Status_Flags_1` bits 7:4).
    pub lane1: LtpReq,
    /// Lane 2 (`Status_Flags_2` bits 3:0).
    pub lane2: LtpReq,
    /// Lane 3 (`Status_Flags_2` bits 7:4). Not in use in 3-lane FRL, where it holds
    /// whatever the sink leaves there (not necessarily `LtpReq::None`); the link training
    /// layer ignores it.
    pub lane3: LtpReq,
}

impl LtpRequests {
    /// Constructs `LtpRequests` from the per-lane requests.
    pub const fn new(lane0: LtpReq, lane1: LtpReq, lane2: LtpReq, lane3: LtpReq) -> Self {
        Self {
            lane0,
            lane1,
            lane2,
            lane3,
        }
    }

    /// Returns `true` when none of the four lanes requests a pattern. In 3-lane FRL lane 3
    /// is not in use and may hold anything, so there only lanes 0–2 decide whether
    /// training passed (as plumbob checks them).
    pub fn all_trained(&self) -> bool {
        [self.lane0, self.lane1, self.lane2, self.lane3]
            .iter()
            .all(|r| *r == LtpReq::None)
    }
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
/// `HdmiForumFrl::NotSupported` turns FRL off (LTS:L). Mirrors culvert's `FrlConfig`;
/// build one with [`new`](Self::new).
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FrlConfig {
    /// FRL rate to request. Use [`HdmiForumFrl::NotSupported`] to clear FRL mode.
    pub frl_rate: HdmiForumFrl,
    /// FFE levels to advertise to the sink: the highest TxFFE level the source will use
    /// at this rate.
    pub ffe_levels: FfeLevels,
}

impl FrlConfig {
    /// Constructs `FrlConfig` from the rate and the FFE levels.
    pub const fn new(frl_rate: HdmiForumFrl, ffe_levels: FfeLevels) -> Self {
        Self {
            frl_rate,
            ffe_levels,
        }
    }
}

/// `Update_0` (0x10): the flags the sink sets to tell the source that something changed.
///
/// Mirrors culvert's `UpdateFlags` field for field. plumbob acts on `source_test_update`,
/// `frl_start` and `flt_update`, and clears only those (see [`ClearableUpdateFlags`]); the
/// other flags are read so that an `ScdcClient` reports the whole register, and are left
/// to whoever handles them. Build one with [`new`](Self::new), or from the default (no flag
/// set) by setting fields.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct UpdateFlags {
    /// Bit 0: general status has changed.
    pub status_update: bool,
    /// Bit 1: CED counters have been updated; re-read the `ERR_DET` registers.
    pub ced_update: bool,
    /// Bit 2: read request test, owned by the sink. The source does not clear it, so
    /// [`ClearableUpdateFlags`] has no field for it.
    pub rr_test: bool,
    /// Bit 3: the sink has written `Source_Test_Configuration` (0x35).
    pub source_test_update: bool,
    /// Bit 4: link training passed; the source may start FRL transmission.
    pub frl_start: bool,
    /// Bit 5: the per-lane link training pattern requests have changed.
    pub flt_update: bool,
    /// Bit 6: the Reed-Solomon correction count has been updated.
    pub rsed_update: bool,
}

impl UpdateFlags {
    /// Constructs `UpdateFlags` from the `Update_0` flags, in bit order.
    pub const fn new(
        status_update: bool,
        ced_update: bool,
        rr_test: bool,
        source_test_update: bool,
        frl_start: bool,
        flt_update: bool,
        rsed_update: bool,
    ) -> Self {
        Self {
            status_update,
            ced_update,
            rr_test,
            source_test_update,
            frl_start,
            flt_update,
            rsed_update,
        }
    }

    /// These flags as a clear: every flag but `RR_Test`, which the source does not clear.
    pub const fn clearable(self) -> ClearableUpdateFlags {
        ClearableUpdateFlags {
            status_update: self.status_update,
            ced_update: self.ced_update,
            source_test_update: self.source_test_update,
            frl_start: self.frl_start,
            flt_update: self.flt_update,
            rsed_update: self.rsed_update,
        }
    }
}

/// The `Update_0` flags a source may clear: every flag but `RR_Test` (bit 2).
///
/// Taken by [`ScdcClient::clear_update_flags`](crate::ScdcClient::clear_update_flags);
/// mirrors culvert's `ClearableUpdateFlags` field for field. plumbob clears only the flags
/// it acts on (`source_test_update`, `frl_start`, `flt_update`). `RR_Test` has no field
/// because it belongs to the sink's read-request test; culvert's architecture document
/// ("RR_Test is not cleared") gives the sources for that rule. The default clears nothing.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ClearableUpdateFlags {
    /// Bit 0: general status has changed.
    pub status_update: bool,
    /// Bit 1: CED counters have been updated.
    pub ced_update: bool,
    /// Bit 3: the sink has written `Source_Test_Configuration` (0x35).
    pub source_test_update: bool,
    /// Bit 4: link training passed.
    pub frl_start: bool,
    /// Bit 5: the per-lane link training pattern requests have changed.
    pub flt_update: bool,
    /// Bit 6: the Reed-Solomon correction count has been updated.
    pub rsed_update: bool,
}

impl ClearableUpdateFlags {
    /// Constructs `ClearableUpdateFlags` from the flags to clear, in bit order.
    pub const fn new(
        status_update: bool,
        ced_update: bool,
        source_test_update: bool,
        frl_start: bool,
        flt_update: bool,
        rsed_update: bool,
    ) -> Self {
        Self {
            status_update,
            ced_update,
            source_test_update,
            frl_start,
            flt_update,
            rsed_update,
        }
    }
}

/// `Source_Test_Configuration` (0x35): instructions a compliance tester writes for the
/// source. Bits 0 and 4 are reserved.
///
/// Mirrors culvert's `SourceTestConfig` field for field; plumbob records all six bits in
/// [`TrainingEvent::SourceTestConfigRead`](crate::TrainingEvent::SourceTestConfigRead).
/// plumbob acts on `flt_no_timeout`, and passes the three TxFFE bits to the PHY with
/// every equalization (as [`TxFfeRestrictions`](crate::TxFfeRestrictions), re-sent when a
/// `Source_Test_Update` seen in LTS:3 or LTS:P changes them). `frl_max` and
/// `dsc_frl_max` ask the source to train at its highest rate; which rates to train at is
/// the caller's choice, so the layer that builds the rate list reads this register
/// (through its SCDC implementation) before training and honours them there. Build one
/// with [`new`](Self::new), or from the default (no bit set) by setting fields.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SourceTestConfig {
    /// Bit 1: use pre-shoot only for TxFFE.
    pub txffe_pre_shoot_only: bool,
    /// Bit 2: use de-emphasis only for TxFFE.
    pub txffe_de_emphasis_only: bool,
    /// Bit 3: use no TxFFE.
    pub txffe_no_ffe: bool,
    /// Bit 5: the source should not time out link training. plumbob then suspends the
    /// LTS:2, LTS:3 and LTS:P poll limits (up to `no_timeout_poll_cap`).
    pub flt_no_timeout: bool,
    /// Bit 6: the source should use the maximum FRL rate with DSC. With `frl_max` also
    /// set, the Amlogic (citing the specification) and NVIDIA drivers act on neither; the
    /// AMD driver trains at the maximum rate.
    pub dsc_frl_max: bool,
    /// Bit 7: the source should use the maximum FRL rate. With `dsc_frl_max` also set, the
    /// Amlogic (citing the specification) and NVIDIA drivers act on neither; the AMD
    /// driver trains at the maximum rate.
    pub frl_max: bool,
}

impl SourceTestConfig {
    /// Constructs `SourceTestConfig` from its flags, in bit order.
    pub const fn new(
        txffe_pre_shoot_only: bool,
        txffe_de_emphasis_only: bool,
        txffe_no_ffe: bool,
        flt_no_timeout: bool,
        dsc_frl_max: bool,
        frl_max: bool,
    ) -> Self {
        Self {
            txffe_pre_shoot_only,
            txffe_de_emphasis_only,
            txffe_no_ffe,
            flt_no_timeout,
            dsc_frl_max,
            frl_max,
        }
    }
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
/// populated in 4-lane FRL mode. Mirrors culvert's `CedCounters`; build one with
/// [`new`](Self::new).
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CedCounters {
    /// Character error count for lane 0, or `None` if the validity bit is not set.
    pub lane0: Option<CedCount>,
    /// Character error count for lane 1, or `None` if the validity bit is not set.
    pub lane1: Option<CedCount>,
    /// Character error count for lane 2, or `None` if the validity bit is not set.
    pub lane2: Option<CedCount>,
    /// Character error count for lane 3, or `None` if the validity bit is not set.
    /// Always `None` in TMDS mode or 3-lane FRL mode.
    pub lane3: Option<CedCount>,
}

impl CedCounters {
    /// Constructs a `CedCounters`.
    pub const fn new(
        lane0: Option<CedCount>,
        lane1: Option<CedCount>,
        lane2: Option<CedCount>,
        lane3: Option<CedCount>,
    ) -> Self {
        Self {
            lane0,
            lane1,
            lane2,
            lane3,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // --- LtpReq ---

    #[test]
    fn ltp_req_from_value_gives_one_request_per_value() {
        for value in 0u8..=0xF {
            let request = LtpReq::from_value(value).unwrap();
            assert_eq!(request.value(), value, "value {value:#x}");
            assert_eq!(
                matches!(request, LtpReq::Reserved(_)),
                (0x9..=0xD).contains(&value),
                "value {value:#x}"
            );
        }
        assert_eq!(LtpReq::from_value(0x10), None);
        assert_eq!(LtpReq::from_value(0xFF), None);
    }

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

    // --- UpdateFlags, ClearableUpdateFlags ---

    #[test]
    fn update_flags_new_takes_the_flags_in_bit_order() {
        let bit = |n: usize| {
            let b = |i| i == n;
            UpdateFlags::new(b(0), b(1), b(2), b(3), b(4), b(5), b(6))
        };
        let set = |f: UpdateFlags| {
            [
                f.status_update,
                f.ced_update,
                f.rr_test,
                f.source_test_update,
                f.frl_start,
                f.flt_update,
                f.rsed_update,
            ]
        };
        for n in 0..7 {
            let expected: [bool; 7] = core::array::from_fn(|i| i == n);
            assert_eq!(set(bit(n)), expected, "bit {n}");
        }
        assert_eq!(UpdateFlags::default(), bit(7));
    }

    #[test]
    fn clearable_keeps_every_flag_but_rr_test() {
        let all = UpdateFlags::new(true, true, true, true, true, true, true);
        assert_eq!(
            all.clearable(),
            ClearableUpdateFlags::new(true, true, true, true, true, true)
        );
        let rr_test_only = UpdateFlags::new(false, false, true, false, false, false, false);
        assert_eq!(rr_test_only.clearable(), ClearableUpdateFlags::default());
    }

    #[test]
    fn source_test_config_new_takes_the_bits_in_bit_order() {
        let set = |c: SourceTestConfig| {
            [
                c.txffe_pre_shoot_only,
                c.txffe_de_emphasis_only,
                c.txffe_no_ffe,
                c.flt_no_timeout,
                c.dsc_frl_max,
                c.frl_max,
            ]
        };
        for n in 0..6 {
            let b = |i| i == n;
            let expected: [bool; 6] = core::array::from_fn(|i| i == n);
            assert_eq!(
                set(SourceTestConfig::new(b(0), b(1), b(2), b(3), b(4), b(5))),
                expected,
                "field {n}"
            );
        }
        assert_eq!(set(SourceTestConfig::default()), [false; 6]);
    }

    #[test]
    fn clearable_update_flags_new_takes_the_flags_in_bit_order() {
        let c = ClearableUpdateFlags::new(true, false, true, false, true, false);
        assert!(c.status_update && c.source_test_update && c.flt_update);
        assert!(!c.ced_update && !c.frl_start && !c.rsed_update);
    }

    // --- LtpRequests, CedCounters, FrlConfig constructors ---

    #[test]
    fn ltp_requests_new_takes_the_lanes_in_order() {
        let r = LtpRequests::new(LtpReq::Lfsr0, LtpReq::Lfsr1, LtpReq::Lfsr2, LtpReq::Lfsr3);
        assert_eq!(
            [r.lane0, r.lane1, r.lane2, r.lane3],
            [LtpReq::Lfsr0, LtpReq::Lfsr1, LtpReq::Lfsr2, LtpReq::Lfsr3]
        );
    }

    #[test]
    fn all_trained_checks_all_four_lanes() {
        let none = LtpReq::None;
        assert!(LtpRequests::new(none, none, none, none).all_trained());
        assert!(!LtpRequests::new(LtpReq::Lfsr0, none, none, none).all_trained());
        // Lane 3 counts, even though it is not in use in 3-lane FRL.
        assert!(!LtpRequests::new(none, none, none, LtpReq::Reserved(0x9)).all_trained());
    }

    #[test]
    fn ced_counters_new_takes_the_lanes_in_order() {
        let c = CedCounters::new(
            Some(CedCount::new(0)),
            Some(CedCount::new(1)),
            Some(CedCount::new(2)),
            None,
        );
        assert_eq!(c.lane0, Some(CedCount::new(0)));
        assert_eq!(c.lane1, Some(CedCount::new(1)));
        assert_eq!(c.lane2, Some(CedCount::new(2)));
        assert_eq!(c.lane3, None);
    }

    #[test]
    fn frl_config_new_takes_the_rate_and_levels() {
        let c = FrlConfig::new(HdmiForumFrl::Rate6Gbps4Lanes, FfeLevels::new(2).unwrap());
        assert_eq!(c.frl_rate, HdmiForumFrl::Rate6Gbps4Lanes);
        assert_eq!(c.ffe_levels, FfeLevels::new(2).unwrap());
    }
}
