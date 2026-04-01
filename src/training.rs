use display_types::cea861::hdmi_forum::HdmiForumFrl;

use crate::types::FfeLevels;

/// Outcome of a training attempt at a single FRL rate.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrainingOutcome {
    /// All lanes satisfied. The link is ready at this rate.
    Success {
        /// The FRL rate at which training succeeded.
        achieved_rate: HdmiForumFrl,
    },
    /// Training did not converge within the configured timeout.
    ///
    /// The caller should retry at a lower rate or fall back to TMDS.
    FallbackRequired,
}

/// Hard error that terminated a training attempt.
///
/// Distinct from [`TrainingOutcome::FallbackRequired`]: this means something
/// failed at the I/O level, not that the link simply did not train at this rate.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrainingError<ScdcErr, PhyErr> {
    /// The `ScdcClient` returned an error.
    Scdc(ScdcErr),
    /// The PHY returned an error.
    Phy(PhyErr),
}

/// Per-attempt training configuration.
///
/// Construct via [`TrainingConfig::default`] and override individual fields as
/// needed for your polling cadence and hardware constraints.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TrainingConfig {
    /// FFE levels advertised to the sink in Config_0.
    pub ffe_levels: FfeLevels,
    /// Whether to set DSC_FRL_Max in Config_0.
    ///
    /// Set to `true` when the negotiated configuration requires DSC transport.
    /// plumbob passes this through into `FrlConfig` without interpreting it.
    /// Defaults to `false`.
    pub dsc_frl_max: bool,
    /// Maximum poll iterations waiting for `flt_ready` (phase 2).
    pub flt_ready_timeout: u32,
    /// Maximum poll iterations waiting for `frl_start` (phase 3).
    pub frl_start_timeout: u32,
    /// Maximum poll iterations in the LTP training loop (phase 4).
    pub ltp_timeout: u32,
}

impl Default for TrainingConfig {
    fn default() -> Self {
        Self {
            ffe_levels: FfeLevels::Ffe0,
            dsc_frl_max: false,
            flt_ready_timeout: 1000,
            frl_start_timeout: 1000,
            ltp_timeout: 1000,
        }
    }
}

// FrlTrainer is added in step 6.

#[cfg(test)]
mod tests {
    use super::*;
    use display_types::cea861::hdmi_forum::HdmiForumFrl;

    // --- TrainingConfig::default ---

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
        let cfg = TrainingConfig::default();
        assert_eq!(cfg.flt_ready_timeout, 1000);
        assert_eq!(cfg.frl_start_timeout, 1000);
        assert_eq!(cfg.ltp_timeout, 1000);
    }

    // --- TrainingOutcome ---

    #[test]
    fn training_outcome_success_carries_rate() {
        let outcome = TrainingOutcome::Success {
            achieved_rate: HdmiForumFrl::Rate6Gbps4Lanes,
        };
        assert_eq!(
            outcome,
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

    // --- TrainingError ---

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
        let scdc: TrainingError<u8, u8> = TrainingError::Scdc(1);
        let phy: TrainingError<u8, u8> = TrainingError::Phy(1);
        assert_ne!(scdc, phy);
    }
}
