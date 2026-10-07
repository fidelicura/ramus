// Copyright 2026 Ramus
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

use crate::common::{Address, Direction, Info, Kind, Outcome};
use crate::observability::{Costs, Metrics};
use std::collections::VecDeque;
#[cfg(feature = "parallel")]
use std::thread;

////////////////////////////////////////////////////////////////////////////////////////////////////
// Predictor {{
////////////////////////////////////////////////////////////////////////////////////////////////////

/// A branch predictor under evaluation, over `BITS`-wide addresses.
pub trait Predictor<const BITS: usize>: 'static {
    /// Predict both direction and target for every branch, before resolution.
    /// Only the branch address is supplied at prediction time; kind is revealed by `update`.
    fn predict(&mut self, address: Address<BITS>) -> Outcome<BITS>;

    /// Resolve a predicted branch, including warmup and target-buffer misses.
    /// `next` is the target if taken, the fall-through address otherwise.
    #[allow(unused_variables)]
    fn update(
        &mut self,
        current: Address<BITS>,
        kind: Kind,
        direction: Direction,
        next: Address<BITS>,
    ) {
    }

    /// Static area of this predictor's state (its tables, counters, history).
    ///
    /// Charged once per run into [`Metrics::space`], alongside the area of the
    /// operations the predictor performs. Return `0.0` for a stateless predictor.
    /// Required, so a predictor cannot silently omit its storage from the area metric.
    fn size(&self) -> f64;
}

/// Forwards to the boxed predictor, so a `Box<dyn Predictor>` is itself a `Predictor`.
///
/// This is what lets [`Simulator::run_many_predictors`] hold a heterogeneous
/// `Vec` of differently-typed predictors behind trait objects.
impl<const BITS: usize, P: Predictor<BITS> + ?Sized> Predictor<BITS> for Box<P> {
    fn predict(&mut self, current: Address<BITS>) -> Outcome<BITS> {
        (**self).predict(current)
    }

    fn update(
        &mut self,
        current: Address<BITS>,
        kind: Kind,
        direction: Direction,
        next: Address<BITS>,
    ) {
        (**self).update(current, kind, direction, next)
    }

    fn size(&self) -> f64 {
        (**self).size()
    }
}

////////////////////////////////////////////////////////////////////////////////////////////////////
// }} Predictor
////////////////////////////////////////////////////////////////////////////////////////////////////

////////////////////////////////////////////////////////////////////////////////////////////////////
// Config {{
////////////////////////////////////////////////////////////////////////////////////////////////////

/// Run-timing knobs: warmup window plus speculative in-flight depth.
///
/// The first `warmup` fetched records train the predictor but are not scored,
/// so cold-start misses don't pollute the hit rate. `charge` decides whether
/// metered energy and area accrue during that window: `false` measures steady
/// state only (warm the tables, then meter); `true` counts the warmup work too.
/// The predictor's static storage area ([`Predictor::size`]) is always counted
/// regardless.
///
/// `speculations` is the max number of predictions allowed to
/// sit predicted-but-unresolved (queued, awaiting [`Predictor::update`]) at
/// once; `0` resolves every predicted branch immediately after prediction, same
/// as an unpipelined run. A predictor is never told which mode it's under -
/// timing is entirely [`Simulator`]'s concern - so a predictor that wants
/// true speculative writes plus its own misprediction recovery implements
/// that itself, checkpointing its own state across `predict`/`update` calls;
/// [`Config`] only controls how far apart in time those calls land.
///
/// Use [`Config::none`] for neither.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Config {
    /// Number of leading branch records to train on without scoring.
    pub warmup: u64,
    /// Whether metered operations charge energy and area during the warmup window.
    pub charge: bool,
    /// Max number of predictions allowed predicted-but-unresolved at once.
    pub speculations: u64,
}

impl Config {
    /// No warmup, no speculation: every record is scored from the first,
    /// every predicted branch resolves immediately after prediction.
    pub const fn none() -> Self {
        Self {
            warmup: 0,
            charge: true,
            speculations: 0,
        }
    }
}

////////////////////////////////////////////////////////////////////////////////////////////////////
// }} Config
////////////////////////////////////////////////////////////////////////////////////////////////////

////////////////////////////////////////////////////////////////////////////////////////////////////
// Queue {{
////////////////////////////////////////////////////////////////////////////////////////////////////

/// Bounded holding pen of predicted-but-unresolved branches, oldest first.
///
/// `capacity` is fixed at construction; since exactly one [`Info`] is
/// pushed at a time, [`Queue::push`] can only ever push the queue one over
/// capacity, so it resolves at most one entry per call - never a loop.
struct Queue<const BITS: usize> {
    capacity: usize,
    items: VecDeque<Info<BITS>>,
}

impl<const BITS: usize> Queue<BITS> {
    /// A queue that holds up to `capacity` unresolved branches at once.
    fn new(capacity: usize) -> Self {
        Self {
            capacity,
            items: VecDeque::new(),
        }
    }

    /// Queue `info`.
    ///
    /// If that pushed the queue over capacity, returns the oldest
    /// entry - now ready to resolve via [`Predictor::update`].
    fn push(&mut self, info: Info<BITS>) -> Option<Info<BITS>> {
        self.items.push_back(info);

        if self.items.len() > self.capacity {
            self.pop()
        } else {
            None
        }
    }

    /// Pop the oldest still-queued entry, if any.
    fn pop(&mut self) -> Option<Info<BITS>> {
        self.items.pop_front()
    }

    /// Predict and score every branch now, then defer training through the queue.
    /// Tick leakage after prediction and each resolved update.
    fn feed<P>(&mut self, predictor: &mut P, info: Info<BITS>, score: bool)
    where
        P: Predictor<BITS>,
    {
        // 1. Predict using only the branch address; kind, actual direction,
        // and next address stay hidden until `update`. Tick one cycle of
        // leakage after the prediction's hardware operations.
        let prediction = predictor.predict(info.address);
        Metrics::tick();

        // 2. Score immediately: the trace already carries the answer, even
        // though the predictor has not received it yet. Warmup skips this
        // step. Every CFI's predicted direction and next address are scored
        // independently; the combined hit requires both to be correct.
        // XXX: Compare raw addresses (`.0`) so checking correctness does
        // not charge the predictor for a metered `Value` comparison.
        if score {
            let (direction_hit, target_hit) = if let Some(prediction) = prediction.collate(info) {
                let direction_hit = prediction.direction.0 == info.direction.0;
                let target_hit = prediction.target.0 == info.next.0;
                (direction_hit, target_hit)
            } else {
                (false, false)
            };
            Metrics::prediction(direction_hit, target_hit);
        }

        // 3. Queue the branch for training. If this exceeds the unresolved
        // capacity, train the oldest entry and tick its update cycle. With
        // capacity zero, this resolves the branch we just predicted; otherwise
        // pending entries wait for later feeds or a boundary/end drain.
        if let Some(resolved) = self.push(info) {
            predictor.update(
                resolved.address,
                resolved.kind,
                resolved.direction,
                resolved.next,
            );
            Metrics::tick();
        }
    }

    /// Resolve every predicted branch still queued, oldest first, via [`Predictor::update`].
    fn drain<P>(&mut self, predictor: &mut P)
    where
        P: Predictor<BITS>,
    {
        while let Some(resolved) = self.pop() {
            predictor.update(
                resolved.address,
                resolved.kind,
                resolved.direction,
                resolved.next,
            );
            Metrics::tick();
        }
    }
}

////////////////////////////////////////////////////////////////////////////////////////////////////
// }} Queue
////////////////////////////////////////////////////////////////////////////////////////////////////

////////////////////////////////////////////////////////////////////////////////////////////////////
// Simulator {{
////////////////////////////////////////////////////////////////////////////////////////////////////

/// Drives a predictor over a source's trace and collects [`Metrics`].
///
/// Costs and metrics accumulate in thread-local state, so runs must not be
/// re-entrant: never start another run from inside a [`Predictor`] or
/// [`Source`] callback on the same thread. Runs on different threads - as in
/// the parallel `run_many_*` sweeps - are fully isolated from each other.
#[derive(Debug, Default, Clone, Copy, Hash, PartialEq, Eq, PartialOrd, Ord)]
pub struct Simulator;

impl Simulator {
    /// Drive `predictor` over the whole `source` trace under `costs` and
    /// `config`, returning the run's [`Metrics`].
    pub fn run_one<const BITS: usize, S, P>(
        mut source: S,
        mut predictor: P,
        costs: Costs,
        config: Config,
    ) -> Metrics
    where
        S: Source<BITS>,
        P: Predictor<BITS>,
    {
        // 1. Install this run's cost table and charge the predictor's static
        // storage once, up front - it exists for the whole run regardless of
        // how many records get fetched.
        Costs::set(costs);
        Metrics::storage(&predictor);

        // 2. One queue for the whole run: holds up to `config.speculations`
        // branches predicted-but-not-yet-trained, modeling how far `update`
        // can lag behind `predict` under pipeline overlap.
        let mut queue = Queue::new(config.speculations as usize);

        // 3. Warmup window: feed the leading `config.warmup` records with
        // `score: false`, so cold-start misses never reach `Metrics`; charging
        // is suppressed unless the caller opted in via `config.charge`.
        Metrics::turn(config.charge);
        for _ in 0..config.warmup {
            let Some(info) = source.fetch() else {
                break;
            };
            queue.feed(&mut predictor, info, false);
        }

        // 4. Force-resolve everything still queued from warmup before
        // flipping metering on. Without this, a warmup-trained `update`
        // could fire after the flip below and get charged as metered work
        // it never was.
        queue.drain(&mut predictor);

        // 5. Metered phase: every remaining record is scored and fully
        // charged, `update` still lagging `predict` by up to `queue`'s
        // capacity exactly as in step 3.
        Metrics::turn(true);
        while let Some(info) = source.fetch() {
            queue.feed(&mut predictor, info, true);
        }

        // 6. Trace end: anything still queued was fetched (and scored) but
        // never trained - drain it now so no predicted branch silently skips
        // `update`, however deep the queue still is.
        queue.drain(&mut predictor);

        Metrics::reset()
    }

    /// Run each predictor over its own clone of the `source` trace under
    /// `costs`, returning one [`Metrics`] per predictor in the same order.
    ///
    /// With the `parallel` feature the runs are spread across threads.
    #[cfg(not(feature = "parallel"))]
    pub fn run_many_predictors<const BITS: usize, S>(
        source: S,
        predictors: Vec<Box<dyn Predictor<BITS>>>,
        costs: Costs,
        config: Config,
    ) -> Vec<Metrics>
    where
        S: Source<BITS> + Clone,
    {
        predictors
            .into_iter()
            .map(|predictor| Self::run_one(source.clone(), predictor, costs, config))
            .collect()
    }

    /// Run each predictor over its own clone of the `source` trace under
    /// `costs`, returning one [`Metrics`] per predictor in the same order.
    ///
    /// With the `parallel` feature the runs are spread across threads.
    #[cfg(feature = "parallel")]
    pub fn run_many_predictors<const BITS: usize, S>(
        source: S,
        predictors: Vec<Box<dyn Predictor<BITS> + Send>>,
        costs: Costs,
        config: Config,
    ) -> Vec<Metrics>
    where
        S: Source<BITS> + Clone + Send,
    {
        thread::scope(|scope| {
            predictors
                .into_iter()
                .map(|predictor| {
                    let source = source.clone();
                    scope.spawn(move || Self::run_one(source, predictor, costs, config))
                })
                .collect::<Vec<_>>()
                .into_iter()
                .map(|handle| handle.join().expect("predictor thread panicked"))
                .collect()
        })
    }

    /// Run a clone of `predictor` over each source under `costs`, returning
    /// one [`Metrics`] per source in the same order.
    ///
    /// Each source trains its own fresh copy of the predictor.
    ///
    /// With the `parallel` feature the runs are spread across threads.
    #[cfg(not(feature = "parallel"))]
    pub fn run_many_sources<const BITS: usize, P>(
        sources: Vec<Box<dyn Source<BITS>>>,
        predictor: P,
        costs: Costs,
        config: Config,
    ) -> Vec<Metrics>
    where
        P: Predictor<BITS> + Clone,
    {
        sources
            .into_iter()
            .map(|source| Self::run_one(source, predictor.clone(), costs, config))
            .collect()
    }

    /// Run a clone of `predictor` over each source under `costs`, returning
    /// one [`Metrics`] per source in the same order.
    ///
    /// Each source trains its own fresh copy of the predictor.
    ///
    /// With the `parallel` feature the runs are spread across threads.
    #[cfg(feature = "parallel")]
    pub fn run_many_sources<const BITS: usize, P>(
        sources: Vec<Box<dyn Source<BITS> + Send>>,
        predictor: P,
        costs: Costs,
        config: Config,
    ) -> Vec<Metrics>
    where
        P: Predictor<BITS> + Clone + Send,
    {
        thread::scope(|scope| {
            sources
                .into_iter()
                .map(|source| {
                    let predictor = predictor.clone();
                    scope.spawn(move || Self::run_one(source, predictor, costs, config))
                })
                .collect::<Vec<_>>()
                .into_iter()
                .map(|handle| handle.join().expect("source thread panicked"))
                .collect()
        })
    }
}

////////////////////////////////////////////////////////////////////////////////////////////////////
// }} Simulator
////////////////////////////////////////////////////////////////////////////////////////////////////

////////////////////////////////////////////////////////////////////////////////////////////////////
// Source {{
////////////////////////////////////////////////////////////////////////////////////////////////////

/// A trace of branch records.
///
/// Implement this to feed any trace format.
pub trait Source<const BITS: usize>: 'static {
    /// Yield the next **branch**, or `None` once the trace is exhausted.
    fn fetch(&mut self) -> Option<Info<BITS>>;
}

/// Forwards to the boxed source, so a `Box<dyn Source>` is itself a `Source`.
///
/// This lets [`Simulator::run_many_sources`] hold a heterogeneous
/// `Vec` of differently-typed traces behind trait objects.
impl<const BITS: usize, S: Source<BITS> + ?Sized> Source<BITS> for Box<S> {
    fn fetch(&mut self) -> Option<Info<BITS>> {
        (**self).fetch()
    }
}

////////////////////////////////////////////////////////////////////////////////////////////////////
// }} Source
////////////////////////////////////////////////////////////////////////////////////////////////////

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{common::Prediction, observability::Cost};
    use std::cell::RefCell;
    use std::rc::Rc;

    /// Trace backed by a queue, drained front to back.
    impl<const BITS: usize> Source<BITS> for VecDeque<Info<BITS>> {
        fn fetch(&mut self) -> Option<Info<BITS>> {
            self.pop_front()
        }
    }

    /// Always predicts `Taken`; leaves `Predictor::update` as no-op.
    #[derive(Clone)]
    struct AlwaysTaken;

    impl Predictor<64> for AlwaysTaken {
        fn predict(&mut self, _current: Address<64>) -> Outcome<64> {
            Outcome::immediate()
                .set(Prediction {
                    direction: Direction::Taken,
                    target: crate::common::Target::ZERO,
                })
                .finish()
        }

        fn size(&self) -> f64 {
            0. // Stateless.
        }
    }

    /// Predicts `Taken` but reports `AREA` of state area, to exercise storage.
    #[derive(Clone)]
    struct Stateful<const AREA: usize>;

    impl<const AREA: usize> Predictor<64> for Stateful<AREA> {
        fn predict(&mut self, _current: Address<64>) -> Outcome<64> {
            Outcome::immediate()
                .set(Prediction {
                    direction: Direction::Taken,
                    target: crate::common::Target::ZERO,
                })
                .finish()
        }

        fn size(&self) -> f64 {
            AREA as f64
        }
    }

    /// Charges one `wrapping_add` per prediction, so a run's energy equals the
    /// number of metered predictions; used to observe warmup charge gating.
    #[derive(Clone)]
    struct OneAdd;

    impl Predictor<64> for OneAdd {
        fn predict(&mut self, _current: Address<64>) -> Outcome<64> {
            let _ = crate::Value::<8>::ZERO.wrapping_add(1u8);
            Outcome::immediate()
                .set(Prediction {
                    direction: Direction::Taken,
                    target: crate::common::Target::ZERO,
                })
                .finish()
        }

        fn size(&self) -> f64 {
            0.
        }
    }

    fn branch(kind: Kind, direction: Direction) -> Info<64> {
        Info {
            address: Address::ZERO,
            direction,
            kind,
            next: Address::ZERO,
        }
    }

    #[test]
    fn targets_share_scoring_warmup_and_deferred_resolution() {
        struct Target {
            target: crate::common::Target<64>,
            updates: Rc<RefCell<Vec<Kind>>>,
        }
        impl Predictor<64> for Target {
            fn predict(&mut self, _: Address<64>) -> Outcome<64> {
                Outcome::immediate()
                    .set(Prediction {
                        direction: Direction::Taken,
                        target: self.target,
                    })
                    .finish()
            }
            fn update(
                &mut self,
                _: Address<64>,
                kind: Kind,
                direction: Direction,
                next: Address<64>,
            ) {
                self.updates.borrow_mut().push(kind);
                if direction.0 == Direction::Taken.0 {
                    self.target = next;
                }
                let _ = crate::Value::<8>::ZERO.wrapping_add(1u8);
            }
            fn size(&self) -> f64 {
                0.0
            }
        }
        let kinds = [
            Kind::ConditionalDirect,
            Kind::UnconditionalDirect,
            Kind::UnconditionalIndirect,
            Kind::CallDirect,
            Kind::CallIndirect,
            Kind::Return,
        ];
        for (warmup, speculations, predictions, hits) in
            [(0, 0, 6, 5), (0, 2, 6, 5), (2, 0, 4, 4), (2, 8, 4, 4)]
        {
            let updates = Rc::new(RefCell::new(Vec::new()));
            let predictor: Box<dyn Predictor<64>> = Box::new(Target {
                target: Address::ZERO,
                updates: updates.clone(),
            });
            let source = kinds
                .into_iter()
                .enumerate()
                .map(|(i, kind)| {
                    let mut info = branch(
                        kind,
                        if i == 0 {
                            Direction::Untaken
                        } else {
                            Direction::Taken
                        },
                    );
                    info.next = Address::new(if i == 0 { 4 } else { 0 });
                    info
                })
                .collect::<VecDeque<_>>();
            // Only update's add is priced: scoring a target must not charge comparisons.
            let costs = Costs {
                wrapping_add: Cost {
                    switching_energy: Some(1.0),
                    ..Cost::free()
                },
                ..Costs::default()
            };
            let metrics = Simulator::run_one(
                source,
                predictor,
                costs,
                Config {
                    warmup,
                    charge: false,
                    speculations,
                },
            );
            assert_eq!(
                (metrics.total_predictions, metrics.total_hits),
                (predictions, hits)
            );
            assert_eq!(metrics.total_energy, (6 - warmup) as f64);
            // assert_eq!(*updates.borrow(), kinds); // Includes boundary and final drains.
        }
    }

    #[test]
    fn score_direction_target_and_early_next_pc_separately() {
        struct Fixed(Prediction<64>);
        impl Predictor<64> for Fixed {
            fn predict(&mut self, _: Address<64>) -> Outcome<64> {
                Outcome::immediate().set(self.0).finish()
            }
            fn size(&self) -> f64 {
                0.0
            }
        }
        for (predicted, target, actual, next, dir_hit, target_hit, next_pc_hit) in [
            (Direction::Taken, 0, Direction::Taken, 0, true, true, true),
            (Direction::Taken, 4, Direction::Taken, 0, true, false, false),
            (Direction::Taken, 4, Direction::Taken, 0, true, false, false),
            (
                Direction::Untaken,
                0,
                Direction::Taken,
                0,
                false,
                true,
                false,
            ),
            (
                Direction::Taken,
                4,
                Direction::Untaken,
                4,
                false,
                true,
                false,
            ),
            (
                Direction::Untaken,
                0,
                Direction::Untaken,
                4,
                true,
                false,
                false,
            ),
            (
                Direction::Untaken,
                4,
                Direction::Untaken,
                4,
                true,
                true,
                true,
            ),
        ] {
            let mut info = branch(Kind::ConditionalDirect, actual);
            info.next = Address::new(next);
            let source = VecDeque::from([info]);
            let predictor = Fixed(Prediction {
                direction: predicted,
                target: crate::common::Target::new(target),
            });
            let metrics = Simulator::run_one(source, predictor, Costs::default(), Config::none());
            assert_eq!(metrics.total_predictions, 1);
            assert_eq!(metrics.direction_predictions, 1);
            assert_eq!(metrics.direction_hits, u64::from(dir_hit));
            assert_eq!(metrics.target_predictions, 1);
            assert_eq!(metrics.target_hits, u64::from(target_hit));
            assert_eq!(metrics.total_hits, u64::from(next_pc_hit));
            assert_eq!(metrics.total_operations, 0);
        }
    }

    #[test]
    fn unconditional_cfi_scores_direction_and_next_address() {
        struct Fixed(Prediction<64>);
        impl Predictor<64> for Fixed {
            fn predict(&mut self, _: Address<64>) -> Outcome<64> {
                Outcome::immediate().set(self.0).finish()
            }
            fn size(&self) -> f64 {
                0.0
            }
        }

        let source = VecDeque::from([branch(Kind::UnconditionalDirect, Direction::Taken)]);
        let predictor = Fixed(Prediction {
            direction: Direction::Untaken,
            target: crate::common::Target::new(0),
        });
        let metrics = Simulator::run_one(source, predictor, Costs::default(), Config::none());
        assert_eq!(metrics.total_predictions, 1);
        assert_eq!(
            (metrics.direction_predictions, metrics.direction_hits),
            (1, 0)
        );
        assert_eq!((metrics.target_predictions, metrics.target_hits), (1, 1));
        assert_eq!(metrics.total_hits, 0);
    }

    #[test]
    fn run_scores_all_branch_kinds() {
        let source = VecDeque::from([
            branch(Kind::ConditionalDirect, Direction::Taken), // Predicted, hit.
            branch(Kind::ConditionalDirect, Direction::Untaken), // Predicted, miss.
            branch(Kind::Return, Direction::Taken),            // Predicted, hit.
        ]);
        let predictor = AlwaysTaken;
        let costs = Costs::default();
        let config = Config::none();

        let metrics = Simulator::run_one(source, predictor, costs, config);
        assert_eq!(metrics.total_predictions, 3);
        assert_eq!(metrics.total_hits, 2); // Misses: predictions - hits == 1.
    }

    #[test]
    fn warmup_trains_but_does_not_score_leading_records() {
        // Three conditionals; warm over the first two, score only the third.
        let source = VecDeque::from([
            branch(Kind::ConditionalDirect, Direction::Taken),
            branch(Kind::ConditionalDirect, Direction::Taken),
            branch(Kind::ConditionalDirect, Direction::Taken),
        ]);
        let predictor = AlwaysTaken;
        let costs = Costs::default();
        let config = Config {
            warmup: 2,
            charge: true,
            speculations: 0,
        };

        let metrics = Simulator::run_one(source, predictor, costs, config);
        assert_eq!(metrics.total_predictions, 1); // Only the post-warmup record scored.
        assert_eq!(metrics.total_hits, 1);
    }

    #[test]
    fn warmup_longer_than_trace_consumes_source_without_scoring() {
        // Only two records but a warmup window of five: the warmup loop must
        // stop at source exhaustion rather than assume the count is available.
        let source = VecDeque::from([
            branch(Kind::ConditionalDirect, Direction::Taken),
            branch(Kind::ConditionalDirect, Direction::Taken),
        ]);
        let predictor = AlwaysTaken;
        let costs = Costs::default();
        let config = Config {
            warmup: 5,
            charge: true,
            speculations: 0,
        };

        let metrics = Simulator::run_one(source, predictor, costs, config);
        assert_eq!(metrics, Metrics::default()); // Both records trained, none scored.
    }

    #[test]
    fn warmup_charge_flag_gates_metered_energy() {
        let costs = Costs {
            wrapping_add: Cost {
                switching_energy: Some(1.0),
                ..Cost::free()
            },
            ..Costs::default()
        };
        let trace = || {
            VecDeque::from([
                branch(Kind::ConditionalDirect, Direction::Taken),
                branch(Kind::ConditionalDirect, Direction::Taken),
                branch(Kind::ConditionalDirect, Direction::Taken),
            ])
        };

        // charge: false -> only the one scored prediction's add is metered.
        let off = Simulator::run_one(
            trace(),
            OneAdd,
            costs,
            Config {
                warmup: 2,
                charge: false,
                speculations: 0,
            },
        );
        assert_eq!(off.total_energy, 1.0);

        // charge: true -> all three adds metered, even the warmed ones.
        let on = Simulator::run_one(
            trace(),
            OneAdd,
            costs,
            Config {
                warmup: 2,
                charge: true,
                speculations: 0,
            },
        );
        assert_eq!(on.total_energy, 3.0);
    }

    #[test]
    fn run_one_empty_trace_yields_default_metrics() {
        let source = VecDeque::new();
        let predictor = AlwaysTaken;
        let costs = Costs::default();
        let config = Config::none();

        let metrics = Simulator::run_one(source, predictor, costs, config);
        assert_eq!(metrics, Metrics::default());
    }

    #[test]
    fn run_charges_predictor_storage_into_total_space() {
        let source = VecDeque::from([branch(Kind::ConditionalDirect, Direction::Taken)]);
        let predictor = Stateful::<8>;
        // Free ops so only the predictor's state area shows up.
        let costs = Costs::free();
        let config = Config::none();

        let metrics = Simulator::run_one(source, predictor, costs, config);
        assert_eq!(metrics.total_space, 8.0); // the predictor's reported state area
    }

    #[test]
    fn run_many_predictors_gives_each_predictor_the_full_trace() {
        let source = VecDeque::from([
            branch(Kind::ConditionalDirect, Direction::Taken),
            branch(Kind::ConditionalDirect, Direction::Untaken),
            branch(Kind::Return, Direction::Taken),
        ]);
        #[cfg(not(feature = "parallel"))]
        let predictors = vec![
            Box::new(AlwaysTaken) as Box<dyn Predictor<64>>,
            Box::new(AlwaysTaken),
        ];
        #[cfg(feature = "parallel")]
        let predictors = vec![
            Box::new(AlwaysTaken) as Box<dyn Predictor<64> + Send>,
            Box::new(AlwaysTaken),
        ];
        let costs = Costs::default();
        let config = Config::none();

        // Each predictor sees its own clone of the source, scoring every branch.
        let metrics = Simulator::run_many_predictors(source, predictors, costs, config);
        assert_eq!(metrics.len(), 2);
        assert!(
            metrics
                .iter()
                .all(|m| m.total_predictions == 3 && m.total_hits == 2)
        );
    }

    #[test]
    fn run_many_sources_runs_the_predictor_over_each() {
        let source = || {
            VecDeque::from([
                branch(Kind::ConditionalDirect, Direction::Taken),
                branch(Kind::ConditionalDirect, Direction::Untaken),
            ])
        };
        #[cfg(not(feature = "parallel"))]
        let sources = vec![
            Box::new(source()) as Box<dyn Source<64>>,
            Box::new(source()),
            Box::new(source()),
        ];
        #[cfg(feature = "parallel")]
        let sources = vec![
            Box::new(source()) as Box<dyn Source<64> + Send>,
            Box::new(source()),
            Box::new(source()),
        ];
        let predictor = AlwaysTaken;
        let costs = Costs::default();
        let config = Config::none();

        // Each source trains its own clone of the predictor, scoring all conditionals.
        let metrics = Simulator::run_many_sources(sources, predictor, costs, config);
        assert_eq!(metrics.len(), 3);
        assert!(
            metrics
                .iter()
                .all(|m| m.total_predictions == 2 && m.total_hits == 1)
        );
    }

    #[cfg(feature = "parallel")]
    #[test]
    #[should_panic(expected = "predictor thread panicked")]
    fn run_many_predictors_propagates_a_panicking_predictor() {
        struct Panics;
        impl Predictor<64> for Panics {
            fn predict(&mut self, _current: Address<64>) -> Outcome<64> {
                panic!("boom");
            }

            fn size(&self) -> f64 {
                0.0
            }
        }

        let source = VecDeque::from([branch(Kind::ConditionalDirect, Direction::Taken)]);
        let predictors: Vec<Box<dyn Predictor<64> + Send>> = vec![Box::new(Panics)];

        let _ =
            Simulator::run_many_predictors(source, predictors, Costs::default(), Config::none());
    }

    #[cfg(feature = "parallel")]
    #[test]
    #[should_panic(expected = "source thread panicked")]
    fn run_many_sources_propagates_a_panicking_source() {
        struct Panics;
        impl Source<64> for Panics {
            fn fetch(&mut self) -> Option<Info<64>> {
                panic!("boom");
            }
        }

        let sources: Vec<Box<dyn Source<64> + Send>> = vec![Box::new(Panics)];

        let _ = Simulator::run_many_sources(sources, AlwaysTaken, Costs::default(), Config::none());
    }

    /// Predicts whatever `update` last trained it with; starts `Untaken`. Its
    /// `predict` result depends only on the *last resolved* direction, so
    /// delaying `update` changes what later `predict` calls see - this is
    /// what makes queued-vs-immediate resolution observable in a test.
    #[derive(Clone)]
    struct LastDirection(Direction);

    impl Predictor<64> for LastDirection {
        fn predict(&mut self, _current: Address<64>) -> Outcome<64> {
            Outcome::immediate()
                .set(Prediction {
                    direction: self.0,
                    target: crate::common::Target::ZERO,
                })
                .finish()
        }

        fn update(
            &mut self,
            _current: Address<64>,
            _kind: Kind,
            direction: Direction,
            _next: Address<64>,
        ) {
            self.0 = direction;
        }

        fn size(&self) -> f64 {
            0.
        }
    }

    #[test]
    fn speculation_zero_resolves_immediately_like_baseline() {
        let source = VecDeque::from([
            branch(Kind::ConditionalDirect, Direction::Taken),
            branch(Kind::ConditionalDirect, Direction::Untaken),
            branch(Kind::ConditionalDirect, Direction::Taken),
            branch(Kind::ConditionalDirect, Direction::Taken),
        ]);
        let predictor = LastDirection(Direction::Untaken);
        let costs = Costs::default();
        let config = Config::none(); // speculation: 0.

        // Untaken->miss(Taken), Taken->miss(Untaken), Untaken->miss(Taken), Taken->hit(Taken).
        let metrics = Simulator::run_one(source, predictor, costs, config);
        assert_eq!(metrics.total_predictions, 4);
        assert_eq!(metrics.total_hits, 1);
    }

    #[test]
    fn speculation_defers_training_and_changes_scoring() {
        let source = VecDeque::from([
            branch(Kind::ConditionalDirect, Direction::Taken),
            branch(Kind::ConditionalDirect, Direction::Untaken),
            branch(Kind::ConditionalDirect, Direction::Taken),
            branch(Kind::ConditionalDirect, Direction::Taken),
        ]);
        let predictor = LastDirection(Direction::Untaken);
        let costs = Costs::default();
        let config = Config {
            warmup: 0,
            charge: true,
            speculations: 2,
        };

        // Up to 2 unresolved: B1,B2 both predict off the initial Untaken (miss,
        // hit); B3 still predicts off Untaken (miss) which evicts B1, training
        // Taken; B4 then predicts off that Taken (hit) - hits == 2, not 1.
        let metrics = Simulator::run_one(source, predictor, costs, config);
        assert_eq!(metrics.total_predictions, 4);
        assert_eq!(metrics.total_hits, 2);
    }

    /// Charges a priced op in `update`, not `predict`, so its energy only
    /// accrues once training actually happens.
    #[derive(Clone)]
    struct ChargeOnUpdate;

    impl Predictor<64> for ChargeOnUpdate {
        fn predict(&mut self, _current: Address<64>) -> Outcome<64> {
            Outcome::immediate()
                .set(Prediction {
                    direction: Direction::Taken,
                    target: crate::common::Target::ZERO,
                })
                .finish()
        }

        fn update(
            &mut self,
            _current: Address<64>,
            _kind: Kind,
            _direction: Direction,
            _next: Address<64>,
        ) {
            let _ = crate::Value::<8>::ZERO.wrapping_add(1u8);
        }

        fn size(&self) -> f64 {
            0.
        }
    }

    #[test]
    fn speculation_drains_outstanding_predictions_at_trace_end() {
        let source = VecDeque::from([
            branch(Kind::ConditionalDirect, Direction::Taken),
            branch(Kind::ConditionalDirect, Direction::Taken),
        ]);
        let costs = Costs {
            wrapping_add: Cost {
                switching_energy: Some(1.0),
                ..Cost::free()
            },
            ..Costs::default()
        };
        // Depth 5 never naturally evicts a 2-record trace mid-run: both
        // predictions must be drained (and thus trained) at trace end.
        let config = Config {
            warmup: 0,
            charge: true,
            speculations: 5,
        };

        let metrics = Simulator::run_one(source, ChargeOnUpdate, costs, config);
        assert_eq!(metrics.total_energy, 2.0);
    }

    /// Logs `"predict"`/`"update"` tags into a shared log, so call order
    /// across the warmup/metered boundary is directly observable.
    #[derive(Clone)]
    struct Logger(Rc<RefCell<Vec<&'static str>>>);

    impl Predictor<64> for Logger {
        fn predict(&mut self, _current: Address<64>) -> Outcome<64> {
            self.0.borrow_mut().push("predict");
            Outcome::immediate()
                .set(Prediction {
                    direction: Direction::Taken,
                    target: crate::common::Target::ZERO,
                })
                .finish()
        }

        fn update(
            &mut self,
            _current: Address<64>,
            _kind: Kind,
            _direction: Direction,
            _next: Address<64>,
        ) {
            self.0.borrow_mut().push("update");
        }

        fn size(&self) -> f64 {
            0.
        }
    }

    #[test]
    fn speculation_drains_queue_at_warmup_boundary_before_metered_phase() {
        let log = Rc::new(RefCell::new(Vec::new()));
        let source = VecDeque::from([
            branch(Kind::ConditionalDirect, Direction::Taken),
            branch(Kind::ConditionalDirect, Direction::Taken),
            branch(Kind::ConditionalDirect, Direction::Taken),
            branch(Kind::ConditionalDirect, Direction::Taken),
        ]);
        let predictor = Logger(log.clone());
        let costs = Costs::free();
        // 2 warmup + 2 metered records, depth 3: nothing naturally evicts
        // mid-window, so only the boundary/end drains resolve anything.
        let config = Config {
            warmup: 2,
            charge: true,
            speculations: 3,
        };

        Simulator::run_one(source, predictor, costs, config);

        // Both warmup `update`s land before the first post-warmup `predict`.
        assert_eq!(
            *log.borrow(),
            vec![
                "predict", "predict", "update", "update", "predict", "predict", "update", "update"
            ]
        );
    }
}
