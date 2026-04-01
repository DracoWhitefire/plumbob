use display_types::cea861::hdmi_forum::HdmiForumFrl;
use hdmi_hal::phy::{EqParams, HdmiPhy};

use crate::scdc::ScdcClient;
use crate::types::{FfeLevels, FrlConfig, LtpReq};

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

/// The central training type. Owns an `ScdcClient` and an `HdmiPhy`.
///
/// `FrlTrainer` is reusable across multiple `train_at_rate` calls. A caller
/// performing rate fallback calls `train_at_rate` repeatedly on the same trainer,
/// stepping down through FRL tiers, without reconstructing it between attempts.
/// Use `into_parts` to recover the SCDC client and PHY when training is finished.
pub struct FrlTrainer<C, P> {
    scdc: C,
    phy: P,
}

impl<C: ScdcClient, P: HdmiPhy> FrlTrainer<C, P> {
    /// Constructs a new `FrlTrainer` owning the given SCDC client and PHY.
    pub fn new(scdc: C, phy: P) -> Self {
        Self { scdc, phy }
    }

    /// Consumes the trainer and returns the SCDC client and PHY.
    pub fn into_parts(self) -> (C, P) {
        (self.scdc, self.phy)
    }

    /// Runs the full four-phase FRL training sequence at the given rate.
    ///
    /// Returns [`TrainingOutcome::Success`] when all lanes satisfy their LTP
    /// requests, or [`TrainingOutcome::FallbackRequired`] if any phase times out.
    /// A [`TrainingError`] is returned only on hard I/O failures.
    pub fn train_at_rate(
        &mut self,
        rate: HdmiForumFrl,
        config: &TrainingConfig,
    ) -> Result<TrainingOutcome, TrainingError<C::Error, P::Error>> {
        // Phase 1 — Configuration
        self.scdc
            .write_frl_config(FrlConfig {
                rate,
                ffe_levels: config.ffe_levels,
                dsc_frl_max: config.dsc_frl_max,
            })
            .map_err(TrainingError::Scdc)?;
        self.phy.set_frl_rate(rate).map_err(TrainingError::Phy)?;

        // Phase 2 — Readiness: poll until the sink asserts flt_ready.
        //
        // `i` counts failed reads. At success it equals `after_iterations`; at
        // timeout it equals `iterations_elapsed`. This convention is shared with
        // phases 3 and 4 so the trace layer can record correct counts.
        let mut i = 0u32;
        loop {
            let status = self
                .scdc
                .read_training_status()
                .map_err(TrainingError::Scdc)?;
            if status.flt_ready {
                break;
            }
            i += 1;
            if i >= config.flt_ready_timeout {
                return Ok(TrainingOutcome::FallbackRequired);
            }
        }

        // Phase 3 — Initiation: poll until the sink asserts frl_start.
        let mut i = 0u32;
        loop {
            let status = self
                .scdc
                .read_training_status()
                .map_err(TrainingError::Scdc)?;
            if status.frl_start {
                break;
            }
            i += 1;
            if i >= config.frl_start_timeout {
                return Ok(TrainingOutcome::FallbackRequired);
            }
        }

        // Phase 4 — LTP loop: drive patterns until ltp_req reaches None.
        //
        // read_ced is called on each iteration; it will feed equalization
        // adjustments once EqParams is expanded beyond its current placeholder.
        let mut i = 0u32;
        loop {
            let status = self
                .scdc
                .read_training_status()
                .map_err(TrainingError::Scdc)?;
            if status.ltp_req == LtpReq::None {
                return Ok(TrainingOutcome::Success {
                    achieved_rate: rate,
                });
            }
            let _ced = self.scdc.read_ced().map_err(TrainingError::Scdc)?;
            // Placeholder for phy.send_ltp(pattern) — see open items in architecture.md.
            self.phy
                .adjust_equalization(EqParams::default())
                .map_err(TrainingError::Phy)?;
            i += 1;
            if i >= config.ltp_timeout {
                return Ok(TrainingOutcome::FallbackRequired);
            }
        }
    }
}

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
