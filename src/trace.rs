use display_types::cea861::hdmi_forum::HdmiForumFrl;

use crate::types::{FfeLevels, LtpRequests};

#[cfg(feature = "alloc")]
use crate::training::TrainingConfig;
#[cfg(feature = "alloc")]
use alloc::vec::Vec;

/// A single event in a training attempt, in order from LTS:2 to the terminal state.
///
/// Poll counts include the poll that saw the event (`after_polls`), or equal the limit
/// that ran out (`polls`), so they can be read against the limits in the trace's
/// `TrainingConfig`.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrainingEvent {
    /// The sink's `Source_Test_Configuration` was read (LTS:2 or LTS:3).
    SourceTestConfigRead {
        /// Whether the sink set `FLT_no_timeout`.
        flt_no_timeout: bool,
    },
    /// LTS:2: `FLT_ready` asserted.
    FltReady {
        /// Polls of `FLT_ready`, including the one that saw it.
        after_polls: u32,
    },
    /// LTS:2: `FLT_ready` did not assert.
    FltReadyTimeout {
        /// Polls made: the limit.
        polls: u32,
    },
    /// LTS:2 or LTS:4: `Config_1` written with this rate.
    RateConfigured {
        /// The FRL rate.
        rate: HdmiForumFrl,
        /// The FFE levels advertised, limited for the rate.
        ffe_levels: FfeLevels,
    },
    /// LTS:3: the sink posted new requests (one event per `FLT_update`, not per poll).
    /// The requests that pass training are recorded as [`TrainingPassed`] instead.
    ///
    /// [`TrainingPassed`]: TrainingEvent::TrainingPassed
    LtpRequested {
        /// The requests for all four lanes.
        requests: LtpRequests,
    },
    /// LTS:3: a lane's TxFFE level was raised in response to 0xE.
    FfeRaised {
        /// The lane (0–3).
        lane: u8,
        /// Its new TxFFE level.
        level: u8,
    },
    /// LTS:3: a lane requested a value the specification leaves undefined (0x9–0xD). A
    /// lane in use keeps its pattern and TxFFE level; lane 3 at a 3-lane rate is not in
    /// use, and its requests are not acted on whatever their value.
    UndefinedLtpRequest {
        /// The lane (0–3).
        lane: u8,
        /// The value requested.
        value: u8,
        /// Whether the lane is in use at the current rate.
        in_use: bool,
    },
    /// LTS:3: all active lanes reported 0x0.
    TrainingPassed {
        /// Polls of `Update_0` in this pass through LTS:3, including the one that saw it.
        after_polls: u32,
    },
    /// LTS:4: the sink requested a lower rate; training continues at `to`.
    RateLowered {
        /// The rate the sink rejected.
        from: HdmiForumFrl,
        /// The next rate in the list.
        to: HdmiForumFrl,
    },
    /// LTS:4: the sink requested a lower rate than the last one in the list.
    RatesExhausted,
    /// LTS:3: training did not pass within the poll limit.
    TrainingTimeout {
        /// Polls made: the limit.
        polls: u32,
    },
    /// LTS:P: the sink set `FRL_start` and `FLT_update` together. The retrain wins:
    /// `FRL_start` is cleared, and `RetrainRequested` (or `RetrainsExhausted`) follows.
    FrlStartWithRetrain,
    /// LTS:P: the sink requested retraining (`FLT_update`); training returns to LTS:3.
    RetrainRequested,
    /// LTS:P: the sink requested retraining once more after `max_retrains` retrains.
    RetrainsExhausted {
        /// Retrains made: `max_retrains`.
        retrains: u32,
    },
    /// LTS:P: `FRL_start` asserted. Training succeeded.
    FrlStart {
        /// Polls of `Update_0` in LTS:P, including the one that saw it.
        after_polls: u32,
    },
    /// LTS:P: `FRL_start` did not assert.
    FrlStartTimeout {
        /// Polls made: the limit.
        polls: u32,
    },
    /// LTS:2, LTS:3 or LTS:P: the sink sets `FLT_no_timeout` and the
    /// [`no_timeout_poll_cap`](crate::TrainingConfig::no_timeout_poll_cap) ran out. The
    /// attempt ends with `NoTimeoutHold`, leaving the link as it is. The state is the one
    /// the events before it lead into.
    NoTimeoutCapReached {
        /// Polls made in the state: the cap.
        polls: u32,
    },
    /// LTS:L: the sink and PHY were returned to TMDS.
    ExitedToTmds,
    /// LTS:L: a step failed. Every step was attempted; an end whose steps all succeeded
    /// is in TMDS.
    ExitToTmdsFailed {
        /// An SCDC step (`Config_1`, `FLT_update`) failed.
        scdc: bool,
        /// A PHY step (patterns, rate) failed.
        phy: bool,
    },
}

/// The full record of a training attempt: the rates passed in, the configuration in
/// force (so poll counts can be read against their limits) and the ordered events.
///
/// Requires the `alloc` feature.
#[cfg(feature = "alloc")]
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrainingTrace {
    /// The rates passed in, in order.
    pub rates: Vec<HdmiForumFrl>,
    /// The configuration in force during the attempt.
    pub config: TrainingConfig,
    /// The events, in order from LTS:2 to the terminal state.
    pub events: Vec<TrainingEvent>,
}

#[cfg(feature = "alloc")]
impl TrainingTrace {
    /// Constructs a `TrainingTrace` from its parts.
    ///
    /// This constructor exists so that companion crates (such as `plumbob-async`) can
    /// produce a `TrainingTrace` despite the struct being `#[non_exhaustive]`.
    pub fn new(
        rates: Vec<HdmiForumFrl>,
        config: TrainingConfig,
        events: Vec<TrainingEvent>,
    ) -> Self {
        Self {
            rates,
            config,
            events,
        }
    }
}
