//! Non-fatal anomalies of a training attempt, returned with its outcome.

#[cfg(feature = "alloc")]
use alloc::vec::Vec;

use crate::trace::TrainingEvent;
use crate::training::TrainingOutcome;

/// The most warnings a [`Trained`] holds without the `alloc` feature.
///
/// Repeats of a warning are merged (see [`TrainingWarning`]), so an attempt produces at
/// most five, and none are dropped.
pub const MAX_WARNINGS: usize = 8;

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
/// [`lts::run`](crate::lts::run). Use [`iter_warnings`](Self::iter_warnings) to read the
/// warnings with or without the `alloc` feature. An attempt that ends in a
/// [`TrainingError`](crate::TrainingError) has no `Trained`; its trace records what
/// happened.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(not(feature = "alloc"), derive(Copy))]
pub struct Trained {
    /// The outcome.
    pub outcome: TrainingOutcome,
    /// The warnings (`alloc`).
    #[cfg(feature = "alloc")]
    pub warnings: Vec<TrainingWarning>,
    /// The warnings (bare `no_std`): the first [`num_warnings`](Self::num_warnings) slots.
    #[cfg(not(feature = "alloc"))]
    pub warnings: [Option<TrainingWarning>; MAX_WARNINGS],
    /// How many slots of `warnings` are in use (bare `no_std`).
    #[cfg(not(feature = "alloc"))]
    pub num_warnings: usize,
}

impl Trained {
    pub(crate) fn new(outcome: TrainingOutcome) -> Self {
        Self {
            outcome,
            #[cfg(feature = "alloc")]
            warnings: Vec::new(),
            #[cfg(not(feature = "alloc"))]
            warnings: [None; MAX_WARNINGS],
            #[cfg(not(feature = "alloc"))]
            num_warnings: 0,
        }
    }

    /// The warnings, in the order they first occurred.
    pub fn iter_warnings(&self) -> impl Iterator<Item = &TrainingWarning> {
        #[cfg(feature = "alloc")]
        let warnings = self.warnings.iter();
        #[cfg(not(feature = "alloc"))]
        let warnings = self.warnings[..self.num_warnings].iter().flatten();
        warnings
    }

    fn warnings_mut(&mut self) -> impl Iterator<Item = &mut TrainingWarning> {
        #[cfg(feature = "alloc")]
        let warnings = self.warnings.iter_mut();
        #[cfg(not(feature = "alloc"))]
        let warnings = self.warnings[..self.num_warnings].iter_mut().flatten();
        warnings
    }

    fn push(&mut self, warning: TrainingWarning) {
        #[cfg(feature = "alloc")]
        self.warnings.push(warning);
        #[cfg(not(feature = "alloc"))]
        if let Some(slot) = self.warnings.get_mut(self.num_warnings) {
            *slot = Some(warning);
            self.num_warnings += 1;
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
            let earlier = self.warnings_mut().find_map(|warning| match warning {
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
                    *count += 1;
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
}
