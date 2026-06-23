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

use crate::common::{Address, Info, Kind, Outcome};
use crate::observability::{Costs, Metrics};
use std::collections::VecDeque;
use std::thread;

////////////////////////////////////////////////////////////////////////////////////////////////////
// Predictor {{
////////////////////////////////////////////////////////////////////////////////////////////////////

/// A branch predictor under evaluation.
pub trait Predictor: 'static {
    /// Predict whether the conditional branch at `current` address is taken.
    ///
    /// Called only for conditional branches before [`Self::update`].
    fn predict(&mut self, current: Address) -> Outcome;

    /// Update internal state with a scored conditional branch's actual `outcome`.
    ///
    /// `next` is the next address (target if taken, fall-through otherwise)
    /// for history. Called only for conditional branches after [`Self::predict`].
    #[allow(unused_variables)]
    fn update(&mut self, current: Address, outcome: Outcome, next: Address) {}

    /// Observe a non-conditional branch for history.
    ///
    /// History-based predictors can fold its [`Kind`], `current`, and `next` PC
    /// into global/path history. Never called for conditional branches; do not
    /// train scored tables here.
    #[allow(unused_variables)]
    fn track(&mut self, current: Address, kind: Kind, outcome: Outcome, next: Address) {}

    /// Static area of this predictor's state (its tables, counters, history).
    ///
    /// Charged once per run into [`Metrics::space`], alongside the area of the
    /// operations the predictor performs. Return `0.0` for a stateless predictor.
    /// Required, so a predictor cannot silently omit its storage from the area metric.
    fn size(&self) -> f64;
}

/// Forwards to the boxed predictor, so a `Box<dyn Predictor>` is itself a
/// `Predictor`. This is what lets [`Simulator::run_many_predictors`] hold a
/// heterogeneous `Vec` of differently-typed predictors behind trait objects.
impl<P: Predictor + ?Sized> Predictor for Box<P> {
    fn predict(&mut self, current: Address) -> Outcome {
        (**self).predict(current)
    }

    fn update(&mut self, current: Address, outcome: Outcome, next: Address) {
        (**self).update(current, outcome, next)
    }

    fn track(&mut self, current: Address, kind: Kind, outcome: Outcome, next: Address) {
        (**self).track(current, kind, outcome, next)
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
/// `speculation` is the max number of conditional predictions allowed to
/// sit predicted-but-unresolved (queued, awaiting [`Predictor::update`]) at
/// once; `0` resolves every conditional immediately after prediction, same
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
    /// Max number of conditional predictions allowed predicted-but-unresolved at once.
    pub speculations: u64,
}

impl Config {
    /// No warmup, no speculation: every record is scored from the first,
    /// every conditional resolves immediately after prediction.
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

/// Bounded holding pen of predicted-but-unresolved conditionals, oldest
/// first. `capacity` is fixed at construction; since exactly one [`Info`] is
/// pushed at a time, [`Queue::push`] can only ever push the queue one over
/// capacity, so it resolves at most one entry per call - never a loop.
struct Queue {
    capacity: usize,
    items: VecDeque<Info>,
}

impl Queue {
    /// A queue that holds up to `capacity` unresolved conditionals at once.
    fn new(capacity: usize) -> Self {
        Self {
            capacity,
            items: VecDeque::new(),
        }
    }

    /// Queue `info`. If that pushed the queue over capacity, returns the
    /// oldest entry - now ready to resolve via [`Predictor::update`].
    fn push(&mut self, info: Info) -> Option<Info> {
        self.items.push_back(info);

        if self.items.len() > self.capacity {
            self.pop()
        } else {
            None
        }
    }

    /// Pop the oldest still-queued entry, if any.
    fn pop(&mut self) -> Option<Info> {
        self.items.pop_front()
    }

    /// Feed one record to `predictor`, ticking one cycle of leakage:
    /// conditionals are predicted and scored (if `score`) immediately -
    /// ground truth is already known from the trace - then queued via
    /// [`Self::push`]; [`Predictor::update`] fires only for whatever entry
    /// that push resolves. Everything else only updates history via
    /// [`Predictor::track`], immediately.
    fn feed<P>(&mut self, predictor: &mut P, info: Info, score: bool)
    where
        P: Predictor,
    {
        if info.kind == Kind::Conditional {
            // 1. Predict now; this is the only signal a real predictor gets
            // before the branch resolves.
            let prediction = predictor.predict(info.address);
            Metrics::tick();

            // 2. Score now too, not at resolution: the trace already carries
            // the real outcome, so scoring never needs to wait on `update`.
            score.then(|| Metrics::outcome(prediction == info.outcome));

            // 3. Queue this prediction; only train whatever entry that push
            // resolves (if any) - the rest stay queued, still unresolved.
            if let Some(resolved) = self.push(info) {
                predictor.update(resolved.address, resolved.outcome, resolved.next);
                Metrics::tick();
            }
        } else {
            // Non-conditional: no prediction was ever made, so there is
            // nothing to queue or resolve - just fold it into history now.
            predictor.track(info.address, info.kind, info.outcome, info.next);
            Metrics::tick();
        }
    }

    /// Resolve every conditional still queued, oldest first, via [`Predictor::update`].
    fn drain<P>(&mut self, predictor: &mut P)
    where
        P: Predictor,
    {
        while let Some(resolved) = self.pop() {
            predictor.update(resolved.address, resolved.outcome, resolved.next);
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
    pub fn run_one<S, P>(mut source: S, mut predictor: P, costs: Costs, config: Config) -> Metrics
    where
        S: Source,
        P: Predictor,
    {
        // 1. Install this run's cost table and charge the predictor's static
        // storage once, up front - it exists for the whole run regardless of
        // how many records get fetched.
        Costs::set(costs);
        Metrics::storage(&predictor);

        // 2. One queue for the whole run: holds up to `config.speculation`
        // conditionals predicted-but-not-yet-trained, modeling how far
        // `update` can lag behind `predict` under pipeline overlap.
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
        // never trained - drain it now so no conditional silently skips
        // `update`, however deep the queue still is.
        queue.drain(&mut predictor);

        Metrics::reset()
    }

    /// Run each predictor over its own clone of the `source` trace under
    /// `costs`, returning one [`Metrics`] per predictor in the same order.
    ///
    /// With the `parallel` feature the runs are spread across threads.
    #[cfg(not(feature = "parallel"))]
    pub fn run_many_predictors<S>(
        source: S,
        predictors: Vec<Box<dyn Predictor>>,
        costs: Costs,
        config: Config,
    ) -> Vec<Metrics>
    where
        S: Source + Clone,
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
    pub fn run_many_predictors<S>(
        source: S,
        predictors: Vec<Box<dyn Predictor + Send>>,
        costs: Costs,
        config: Config,
    ) -> Vec<Metrics>
    where
        S: Source + Clone + Send,
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

    /// Run a clone of `predictor` over each source under `costs`, returning one
    /// [`Metrics`] per source in the same order. Each source trains its own
    /// fresh copy of the predictor.
    ///
    /// With the `parallel` feature the runs are spread across threads.
    #[cfg(not(feature = "parallel"))]
    pub fn run_many_sources<P>(
        sources: Vec<Box<dyn Source>>,
        predictor: P,
        costs: Costs,
        config: Config,
    ) -> Vec<Metrics>
    where
        P: Predictor + Clone,
    {
        sources
            .into_iter()
            .map(|source| Self::run_one(source, predictor.clone(), costs, config))
            .collect()
    }

    /// Run a clone of `predictor` over each source under `costs`, returning one
    /// [`Metrics`] per source in the same order. Each source trains its own
    /// fresh copy of the predictor.
    ///
    /// With the `parallel` feature the runs are spread across threads.
    #[cfg(feature = "parallel")]
    pub fn run_many_sources<P>(
        sources: Vec<Box<dyn Source + Send>>,
        predictor: P,
        costs: Costs,
        config: Config,
    ) -> Vec<Metrics>
    where
        P: Predictor + Clone + Send,
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
pub trait Source: 'static {
    /// Yield the next **branch**, or `None` once the trace is exhausted.
    fn fetch(&mut self) -> Option<Info>;
}

/// Forwards to the boxed source, so a `Box<dyn Source>` is itself a `Source`.
/// This lets [`Simulator::run_many_sources`] hold a heterogeneous `Vec` of
/// differently-typed traces behind trait objects.
impl<S: Source + ?Sized> Source for Box<S> {
    fn fetch(&mut self) -> Option<Info> {
        (**self).fetch()
    }
}

////////////////////////////////////////////////////////////////////////////////////////////////////
// }} Source
////////////////////////////////////////////////////////////////////////////////////////////////////

#[cfg(test)]
mod tests {
    use super::*;
    use crate::observability::Cost;
    use std::cell::RefCell;
    use std::rc::Rc;

    /// Trace backed by a queue, drained front to back.
    impl Source for VecDeque<Info> {
        fn fetch(&mut self) -> Option<Info> {
            self.pop_front()
        }
    }

    /// Always predicts `Taken`; leaves `Predictor::{update,track}` as no-op.
    #[derive(Clone)]
    struct AlwaysTaken;

    impl Predictor for AlwaysTaken {
        fn predict(&mut self, _current: Address) -> Outcome {
            Outcome::Taken
        }

        fn size(&self) -> f64 {
            0. // Stateless.
        }
    }

    /// Predicts `Taken` but reports `AREA` of state area, to exercise storage.
    #[derive(Clone)]
    struct Stateful<const AREA: usize>;

    impl<const AREA: usize> Predictor for Stateful<AREA> {
        fn predict(&mut self, _current: Address) -> Outcome {
            Outcome::Taken
        }

        fn size(&self) -> f64 {
            AREA as f64
        }
    }

    /// Charges one `wrapping_add` per prediction, so a run's energy equals the
    /// number of metered predictions; used to observe warmup charge gating.
    #[derive(Clone)]
    struct OneAdd;

    impl Predictor for OneAdd {
        fn predict(&mut self, _current: Address) -> Outcome {
            let _ = crate::Value::<8>::ZERO.wrapping_add(1u8);
            Outcome::Taken
        }

        fn size(&self) -> f64 {
            0.
        }
    }

    fn branch(kind: Kind, outcome: Outcome) -> Info {
        Info {
            address: Address::ZERO,
            outcome,
            kind,
            next: Address::ZERO,
        }
    }

    #[test]
    fn run_scores_only_conditionals() {
        let source = VecDeque::from([
            branch(Kind::Conditional, Outcome::Taken), // Predicted, hit.
            branch(Kind::Conditional, Outcome::Untaken), // Predicted, miss.
            branch(Kind::Return, Outcome::Taken),      // Only tracked.
        ]);
        let predictor = AlwaysTaken;
        let costs = Costs::default();
        let config = Config::none();

        let metrics = Simulator::run_one(source, predictor, costs, config);
        assert_eq!(metrics.predictions, 2);
        assert_eq!(metrics.hits, 1); // Misses: predictions - hits == 1.
    }

    #[test]
    fn warmup_trains_but_does_not_score_leading_records() {
        // Three conditionals; warm over the first two, score only the third.
        let source = VecDeque::from([
            branch(Kind::Conditional, Outcome::Taken),
            branch(Kind::Conditional, Outcome::Taken),
            branch(Kind::Conditional, Outcome::Taken),
        ]);
        let predictor = AlwaysTaken;
        let costs = Costs::default();
        let config = Config {
            warmup: 2,
            charge: true,
            speculations: 0,
        };

        let metrics = Simulator::run_one(source, predictor, costs, config);
        assert_eq!(metrics.predictions, 1); // Only the post-warmup record scored.
        assert_eq!(metrics.hits, 1);
    }

    #[test]
    fn warmup_longer_than_trace_consumes_source_without_scoring() {
        // Only two records but a warmup window of five: the warmup loop must
        // stop at source exhaustion rather than assume the count is available.
        let source = VecDeque::from([
            branch(Kind::Conditional, Outcome::Taken),
            branch(Kind::Conditional, Outcome::Taken),
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
                branch(Kind::Conditional, Outcome::Taken),
                branch(Kind::Conditional, Outcome::Taken),
                branch(Kind::Conditional, Outcome::Taken),
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
        assert_eq!(off.energy, 1.0);

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
        assert_eq!(on.energy, 3.0);
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
        let source = VecDeque::from([branch(Kind::Conditional, Outcome::Taken)]);
        let predictor = Stateful::<8>;
        // Free ops so only the predictor's state area shows up.
        let costs = Costs::free();
        let config = Config::none();

        let metrics = Simulator::run_one(source, predictor, costs, config);
        assert_eq!(metrics.space, 8.0); // the predictor's reported state area
    }

    #[test]
    fn run_many_predictors_gives_each_predictor_the_full_trace() {
        let source = VecDeque::from([
            branch(Kind::Conditional, Outcome::Taken),
            branch(Kind::Conditional, Outcome::Untaken),
            branch(Kind::Return, Outcome::Taken),
        ]);
        #[cfg(not(feature = "parallel"))]
        let predictors = vec![
            Box::new(AlwaysTaken) as Box<dyn Predictor>,
            Box::new(AlwaysTaken),
        ];
        #[cfg(feature = "parallel")]
        let predictors = vec![
            Box::new(AlwaysTaken) as Box<dyn Predictor + Send>,
            Box::new(AlwaysTaken),
        ];
        let costs = Costs::default();
        let config = Config::none();

        // Each predictor sees its own clone of the source, so both score all conditionals.
        let metrics = Simulator::run_many_predictors(source, predictors, costs, config);
        assert_eq!(metrics.len(), 2);
        assert!(metrics.iter().all(|m| m.predictions == 2 && m.hits == 1));
    }

    #[test]
    fn run_many_sources_runs_the_predictor_over_each() {
        let source = || {
            VecDeque::from([
                branch(Kind::Conditional, Outcome::Taken),
                branch(Kind::Conditional, Outcome::Untaken),
            ])
        };
        #[cfg(not(feature = "parallel"))]
        let sources = vec![
            Box::new(source()) as Box<dyn Source>,
            Box::new(source()),
            Box::new(source()),
        ];
        #[cfg(feature = "parallel")]
        let sources = vec![
            Box::new(source()) as Box<dyn Source + Send>,
            Box::new(source()),
            Box::new(source()),
        ];
        let predictor = AlwaysTaken;
        let costs = Costs::default();
        let config = Config::none();

        // Each source trains its own clone of the predictor, scoring all conditionals.
        let metrics = Simulator::run_many_sources(sources, predictor, costs, config);
        assert_eq!(metrics.len(), 3);
        assert!(metrics.iter().all(|m| m.predictions == 2 && m.hits == 1));
    }

    #[cfg(feature = "parallel")]
    #[test]
    #[should_panic(expected = "predictor thread panicked")]
    fn run_many_predictors_propagates_a_panicking_predictor() {
        struct Panics;
        impl Predictor for Panics {
            fn predict(&mut self, _current: Address) -> Outcome {
                panic!("boom");
            }

            fn size(&self) -> f64 {
                0.0
            }
        }

        let source = VecDeque::from([branch(Kind::Conditional, Outcome::Taken)]);
        let predictors: Vec<Box<dyn Predictor + Send>> = vec![Box::new(Panics)];

        let _ =
            Simulator::run_many_predictors(source, predictors, Costs::default(), Config::none());
    }

    #[cfg(feature = "parallel")]
    #[test]
    #[should_panic(expected = "source thread panicked")]
    fn run_many_sources_propagates_a_panicking_source() {
        struct Panics;
        impl Source for Panics {
            fn fetch(&mut self) -> Option<Info> {
                panic!("boom");
            }
        }

        let sources: Vec<Box<dyn Source + Send>> = vec![Box::new(Panics)];

        let _ = Simulator::run_many_sources(sources, AlwaysTaken, Costs::default(), Config::none());
    }

    /// Predicts whatever `update` last trained it with; starts `Untaken`. Its
    /// `predict` result depends only on the *last resolved* outcome, so
    /// delaying `update` changes what later `predict` calls see - this is
    /// what makes queued-vs-immediate resolution observable in a test.
    #[derive(Clone)]
    struct LastOutcome(Outcome);

    impl Predictor for LastOutcome {
        fn predict(&mut self, _current: Address) -> Outcome {
            self.0
        }

        fn update(&mut self, _current: Address, outcome: Outcome, _next: Address) {
            self.0 = outcome;
        }

        fn size(&self) -> f64 {
            0.
        }
    }

    #[test]
    fn speculation_zero_resolves_immediately_like_baseline() {
        let source = VecDeque::from([
            branch(Kind::Conditional, Outcome::Taken),
            branch(Kind::Conditional, Outcome::Untaken),
            branch(Kind::Conditional, Outcome::Taken),
            branch(Kind::Conditional, Outcome::Taken),
        ]);
        let predictor = LastOutcome(Outcome::Untaken);
        let costs = Costs::default();
        let config = Config::none(); // speculation: 0.

        // Untaken->miss(Taken), Taken->miss(Untaken), Untaken->miss(Taken), Taken->hit(Taken).
        let metrics = Simulator::run_one(source, predictor, costs, config);
        assert_eq!(metrics.predictions, 4);
        assert_eq!(metrics.hits, 1);
    }

    #[test]
    fn speculation_defers_training_and_changes_scoring() {
        let source = VecDeque::from([
            branch(Kind::Conditional, Outcome::Taken),
            branch(Kind::Conditional, Outcome::Untaken),
            branch(Kind::Conditional, Outcome::Taken),
            branch(Kind::Conditional, Outcome::Taken),
        ]);
        let predictor = LastOutcome(Outcome::Untaken);
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
        assert_eq!(metrics.predictions, 4);
        assert_eq!(metrics.hits, 2);
    }

    /// Charges a priced op in `update`, not `predict`, so its energy only
    /// accrues once training actually happens.
    #[derive(Clone)]
    struct ChargeOnUpdate;

    impl Predictor for ChargeOnUpdate {
        fn predict(&mut self, _current: Address) -> Outcome {
            Outcome::Taken
        }

        fn update(&mut self, _current: Address, _outcome: Outcome, _next: Address) {
            let _ = crate::Value::<8>::ZERO.wrapping_add(1u8);
        }

        fn size(&self) -> f64 {
            0.
        }
    }

    #[test]
    fn speculation_drains_outstanding_predictions_at_trace_end() {
        let source = VecDeque::from([
            branch(Kind::Conditional, Outcome::Taken),
            branch(Kind::Conditional, Outcome::Taken),
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
        assert_eq!(metrics.energy, 2.0);
    }

    /// Logs `"predict"`/`"update"` tags into a shared log, so call order
    /// across the warmup/metered boundary is directly observable.
    #[derive(Clone)]
    struct Logger(Rc<RefCell<Vec<&'static str>>>);

    impl Predictor for Logger {
        fn predict(&mut self, _current: Address) -> Outcome {
            self.0.borrow_mut().push("predict");
            Outcome::Taken
        }

        fn update(&mut self, _current: Address, _outcome: Outcome, _next: Address) {
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
            branch(Kind::Conditional, Outcome::Taken),
            branch(Kind::Conditional, Outcome::Taken),
            branch(Kind::Conditional, Outcome::Taken),
            branch(Kind::Conditional, Outcome::Taken),
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
