use display_types::cea861::hdmi_forum::HdmiForumFrl;

#[cfg(feature = "alloc")]
use crate::training::TrainingConfig;
use crate::types::{FfeLevels, LtpReq};

#[cfg(feature = "alloc")]
use alloc::vec::Vec;

/// A single event in the FRL training sequence.
///
/// Events are recorded in order from phase 1 through the terminal state.
/// Only transitions in `ltp_req` produce a [`TrainingEvent::LtpPatternRequested`]
/// event — a sink that holds the same pattern for multiple iterations produces
/// one event, not one per poll.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TrainingEvent {
    /// Phase 1: Config_0 was written with this rate and FFE level count.
    RateConfigured {
        /// The FRL rate written to Config_0.
        rate: HdmiForumFrl,
        /// The FFE level count advertised to the sink.
        ffe_levels: FfeLevels,
    },

    /// Phase 2: the sink asserted `flt_ready` after this many poll iterations.
    FltReadyReceived {
        /// Number of failed polls before `flt_ready` was observed.
        after_iterations: u32,
    },

    /// Phase 2: timed out waiting for `flt_ready`.
    FltReadyTimeout {
        /// Number of failed polls before the timeout fired.
        iterations_elapsed: u32,
    },

    /// Phase 3: the sink asserted `frl_start` after this many poll iterations.
    FrlStartReceived {
        /// Number of failed polls before `frl_start` was observed.
        after_iterations: u32,
    },

    /// Phase 3: timed out waiting for `frl_start`.
    FrlStartTimeout {
        /// Number of failed polls before the timeout fired.
        iterations_elapsed: u32,
    },

    /// Phase 4: the sink changed its LTP request to this pattern.
    ///
    /// Recorded each time `ltp_req` transitions to a new value, not on every
    /// poll. The sequence shows how the sink's pattern requests evolved.
    LtpPatternRequested {
        /// The new LTP pattern requested by the sink.
        pattern: LtpReq,
    },

    /// Phase 4: `ltp_req` reached `None` on all lanes. Training succeeded.
    AllLanesSatisfied {
        /// Number of LTP loop iterations before all lanes were satisfied.
        after_iterations: u32,
    },

    /// Phase 4: timed out in the LTP loop before `ltp_req` reached `None`.
    LtpLoopTimeout {
        /// Number of LTP loop iterations before the timeout fired.
        iterations_elapsed: u32,
    },
}

/// Full event log for a single training attempt.
///
/// `TrainingTrace` uses [`Vec`] and requires the `alloc` feature. Use
/// `FrlTrainer::train_at_rate_traced` to obtain one.
///
/// The `config` field is recorded alongside the events so a trace is fully
/// self-describing: timeout counts in events such as
/// `FltReadyTimeout { iterations_elapsed: 47 }` are only meaningful when
/// read against the limit that was configured.
#[cfg(feature = "alloc")]
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrainingTrace {
    /// The FRL rate that was attempted.
    pub rate: HdmiForumFrl,
    /// The configuration in force during this attempt.
    pub config: TrainingConfig,
    /// Ordered event log from phase 1 through the terminal state.
    pub events: Vec<TrainingEvent>,
}

#[cfg(feature = "alloc")]
impl TrainingTrace {
    /// Constructs a `TrainingTrace` from its parts.
    ///
    /// This constructor exists so that companion crates (such as `plumbob-async`) can
    /// produce a `TrainingTrace` despite the struct being `#[non_exhaustive]`.
    pub fn new(rate: HdmiForumFrl, config: TrainingConfig, events: Vec<TrainingEvent>) -> Self {
        Self { rate, config, events }
    }
}
