//! Non-fatal anomalies of a training attempt, returned with its outcome.

#[cfg(feature = "alloc")]
use alloc::vec::Vec;

use crate::trace::TrainingEvent;
use crate::training::TrainingOutcome;

/// The most warnings a [`Trained`] holds without the `alloc` feature.
///
/// Repeats of a warning are merged (see [`TrainingWarning`]), so an attempt produces at
/// most five, and none are dropped.
pub const MAX_WARNINGS: usize = 5;

/// A non-fatal anomaly seen during a training attempt.
///
/// Warnings do not change the outcome: the attempt went on as the procedure says. They
/// report sink behaviour a caller may want to know about, without reading the trace.
/// Repeats are merged, so a sink that posts the same anomaly on every round produces one
/// warning with a count; the trace has each occurrence.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrainingWarning {
    /// A lane requested a value the specification leaves undefined (0x9–0xD). LTS:3
    /// left the lane as it was.
    #[non_exhaustive]
    UndefinedLtpRequest {
        /// The lane (0–3).
        lane: u8,
        /// The last undefined value it requested.
        value: u8,
        /// How many times it requested an undefined value.
        count: u32,
        /// Whether the lane was in use. Lane 3 is not in use at the 3-lane rates; a
        /// request there is not acted on whatever its value.
        in_use: bool,
    },
}

/// The outcome of a training attempt, with the warnings it produced.
///
/// Returned by [`FrlTrainer::train`](crate::FrlTrainer::train) and
/// [`lts::run`](crate::lts::run). Read the warnings with [`warnings`](Self::warnings) or
/// [`iter_warnings`](Self::iter_warnings). The API is the same with and without the
/// `alloc` feature (the storage, a `Vec` or [`MAX_WARNINGS`] inline slots, is private), so
/// code written against one build keeps compiling when something else in the dependency
/// graph enables `alloc`. An attempt that ends in a
/// [`TrainingError`](crate::TrainingError) has no `Trained`; its trace records what
/// happened.
#[non_exhaustive]
#[derive(Clone)]
pub struct Trained {
    /// The outcome.
    pub outcome: TrainingOutcome,
    #[cfg(feature = "alloc")]
    warnings: Vec<TrainingWarning>,
    /// Inline storage: the first `len` slots hold the warnings, the rest [`UNUSED`].
    #[cfg(not(feature = "alloc"))]
    warnings: [TrainingWarning; MAX_WARNINGS],
    #[cfg(not(feature = "alloc"))]
    len: usize,
}

/// Fills the inline slots not in use. A real warning has a count of at least 1.
#[cfg(not(feature = "alloc"))]
const UNUSED: TrainingWarning = TrainingWarning::UndefinedLtpRequest {
    lane: 0,
    value: 0,
    count: 0,
    in_use: false,
};

impl Trained {
    pub(crate) fn new(outcome: TrainingOutcome) -> Self {
        Self {
            outcome,
            #[cfg(feature = "alloc")]
            warnings: Vec::new(),
            #[cfg(not(feature = "alloc"))]
            warnings: [UNUSED; MAX_WARNINGS],
            #[cfg(not(feature = "alloc"))]
            len: 0,
        }
    }

    /// The warnings, in the order they first occurred.
    pub fn warnings(&self) -> &[TrainingWarning] {
        #[cfg(feature = "alloc")]
        let warnings = &self.warnings;
        #[cfg(not(feature = "alloc"))]
        let warnings = &self.warnings[..self.len];
        warnings
    }

    /// The warnings, in the order they first occurred.
    pub fn iter_warnings(&self) -> impl Iterator<Item = &TrainingWarning> {
        self.warnings().iter()
    }

    fn warnings_mut(&mut self) -> &mut [TrainingWarning] {
        #[cfg(feature = "alloc")]
        let warnings = &mut self.warnings;
        #[cfg(not(feature = "alloc"))]
        let warnings = &mut self.warnings[..self.len];
        warnings
    }

    fn push(&mut self, warning: TrainingWarning) {
        #[cfg(feature = "alloc")]
        self.warnings.push(warning);
        #[cfg(not(feature = "alloc"))]
        if let Some(slot) = self.warnings.get_mut(self.len) {
            *slot = warning;
            self.len += 1;
        }
    }

    /// Adds the warning `event` gives rise to, if any, merging it with an earlier one of
    /// the same kind.
    pub(crate) fn observe(&mut self, event: &TrainingEvent) {
        if let TrainingEvent::UndefinedLtpRequest {
            lane,
            value,
            in_use,
        } = *event
        {
            let earlier = self
                .warnings_mut()
                .iter_mut()
                .find_map(|warning| match warning {
                    TrainingWarning::UndefinedLtpRequest {
                        lane: l,
                        value,
                        count,
                        in_use: u,
                    } if *l == lane && *u == in_use => Some((value, count)),
                    _ => None,
                });
            match earlier {
                Some((last, count)) => {
                    *last = value;
                    // A sink can repeat a request for as long as a client lets it.
                    *count = count.saturating_add(1);
                }
                None => self.push(TrainingWarning::UndefinedLtpRequest {
                    lane,
                    value,
                    count: 1,
                    in_use,
                }),
            }
        }
    }
}

// By hand, so that every build compares and prints the outcome and the warnings in use,
// never the unused inline slots.
impl core::fmt::Debug for Trained {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Trained")
            .field("outcome", &self.outcome)
            .field("warnings", &self.warnings())
            .finish()
    }
}

impl PartialEq for Trained {
    fn eq(&self, other: &Self) -> bool {
        self.outcome == other.outcome && self.warnings() == other.warnings()
    }
}

impl Eq for Trained {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::training::FallbackReason;

    fn undefined(lane: u8, value: u8, in_use: bool) -> TrainingEvent {
        TrainingEvent::UndefinedLtpRequest {
            lane,
            value,
            in_use,
        }
    }

    #[test]
    fn the_repeat_count_saturates() {
        let mut trained = Trained::new(TrainingOutcome::FallbackRequired {
            reason: FallbackReason::TrainingTimeout,
        });
        trained.observe(&undefined(0, 0x9, true));
        let TrainingWarning::UndefinedLtpRequest { count, .. } = &mut trained.warnings_mut()[0];
        *count = u32::MAX;
        trained.observe(&undefined(0, 0x9, true));
        assert!(matches!(
            trained.iter_warnings().next(),
            Some(TrainingWarning::UndefinedLtpRequest {
                count: u32::MAX,
                ..
            })
        ));
    }

    #[test]
    fn repeats_on_a_lane_are_merged_with_the_last_value() {
        let mut trained = Trained::new(TrainingOutcome::FallbackRequired {
            reason: FallbackReason::TrainingTimeout,
        });
        trained.observe(&undefined(1, 0x9, true));
        trained.observe(&TrainingEvent::ExitedToTmds);
        trained.observe(&undefined(2, 0xA, true));
        trained.observe(&undefined(1, 0xC, true));
        // Lane 3 out of use (a 3-lane rate) is kept apart from lane 3 in use.
        trained.observe(&undefined(3, 0xB, false));
        trained.observe(&undefined(3, 0xB, true));
        let warnings: [TrainingWarning; 4] = [
            TrainingWarning::UndefinedLtpRequest {
                lane: 1,
                value: 0xC,
                count: 2,
                in_use: true,
            },
            TrainingWarning::UndefinedLtpRequest {
                lane: 2,
                value: 0xA,
                count: 1,
                in_use: true,
            },
            TrainingWarning::UndefinedLtpRequest {
                lane: 3,
                value: 0xB,
                count: 1,
                in_use: false,
            },
            TrainingWarning::UndefinedLtpRequest {
                lane: 3,
                value: 0xB,
                count: 1,
                in_use: true,
            },
        ];
        assert!(trained.iter_warnings().eq(warnings.iter()));
    }

    #[test]
    fn every_distinct_warning_fits() {
        // Lanes 0–2 are always in use; lane 3 can be either: five warnings at most.
        let mut trained = Trained::new(TrainingOutcome::FallbackRequired {
            reason: FallbackReason::TrainingTimeout,
        });
        for (lane, in_use) in [(0, true), (1, true), (2, true), (3, true), (3, false)] {
            for _ in 0..100 {
                trained.observe(&undefined(lane, 0x9, in_use));
            }
        }
        assert_eq!(trained.iter_warnings().count(), 5);
        const { assert!(5 <= MAX_WARNINGS) };
    }

    fn timed_out() -> Trained {
        Trained::new(TrainingOutcome::FallbackRequired {
            reason: FallbackReason::TrainingTimeout,
        })
    }

    #[test]
    fn the_slice_and_the_iterator_agree() {
        let mut trained = timed_out();
        assert!(trained.warnings().is_empty());
        trained.observe(&undefined(1, 0x9, true));
        trained.observe(&undefined(2, 0xA, true));
        assert_eq!(trained.warnings().len(), 2);
        assert!(trained.iter_warnings().eq(trained.warnings().iter()));
    }

    #[test]
    fn equality_compares_the_outcome_and_the_warnings_in_use() {
        let (mut a, mut b) = (timed_out(), timed_out());
        assert_eq!(a, b);
        a.observe(&undefined(1, 0x9, true));
        assert_ne!(a, b);
        b.observe(&undefined(1, 0x9, true));
        assert_eq!(a, b);
        let c = Trained::new(TrainingOutcome::FallbackRequired {
            reason: FallbackReason::RatesExhausted,
        });
        assert_ne!(timed_out(), c);
    }

    #[test]
    fn debug_shows_the_outcome_and_only_the_warnings_in_use() {
        extern crate std;
        use std::format;
        let mut trained = timed_out();
        trained.observe(&undefined(1, 0x9, true));
        let debug = format!("{trained:?}");
        assert!(debug.starts_with("Trained { outcome: FallbackRequired"));
        assert_eq!(debug.matches("UndefinedLtpRequest").count(), 1);
    }
}
