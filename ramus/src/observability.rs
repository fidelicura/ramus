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

use crate::simulator::Predictor;
use std::cell::{Cell, RefCell};
use std::collections::HashSet;

////////////////////////////////////////////////////////////////////////////////////////////////////
// Ledger {{
////////////////////////////////////////////////////////////////////////////////////////////////////

thread_local! {
    /// Per-thread run state that [`Value`](crate::Value) operations and
    /// predictors write into without threading a context argument through every call.
    ///
    /// Thread-locality keeps concurrent simulation runs - including parallel
    /// tests - isolated.
    static LEDGER: RefCell<Ledger> = RefCell::new(Ledger::new());
}

/// Everything one simulation run accumulates.
///
/// The public [`Metrics`] totals plus the bookkeeping needed to compute them,
/// held in one cell so [`Metrics::reset`] clears all of it in a single step
/// instead of juggling several thread-locals in lockstep.
#[derive(Debug, Clone, Default)]
struct Ledger {
    metrics: Metrics,
    /// Distinct `(Operation, size)` pairs whose static space has already been counted this run.
    ///
    /// `size` is whatever distinguishes one instance of that operation's hardware
    /// from another (`BITS` width for arithmetic, `ways` for a multiplexer), so a
    /// `wrapping_add::<8>` and a `wrapping_add::<64>`, or a 2-way and an 8-way
    /// `select!`, are charged as the separate hardware elements they are.
    charged: HashSet<(Operation, usize)>,
    /// Sum of `leakage_energy` for every distinct `(Operation, size)` charged
    /// so far - the hardware instantiated this run.
    ///
    /// Ticked into [`Metrics::leakage`] once per cycle by [`Metrics::tick`],
    /// since leakage drains continuously however often ops fire.
    leakage: f64,
    /// Greatest work time charged inside each active overlap group, outermost first.
    overlaps: Vec<u64>,
    /// Whether metered operations currently accrue.
    ///
    /// The simulator clears this during a warmup window with charging
    /// off, so warmup work runs but is not metered.
    charging: bool,
}

impl Ledger {
    fn new() -> Self {
        Self {
            charging: true,
            ..Self::default()
        }
    }
}

////////////////////////////////////////////////////////////////////////////////////////////////////
// }} Ledger
////////////////////////////////////////////////////////////////////////////////////////////////////

////////////////////////////////////////////////////////////////////////////////////////////////////
// Metrics {{
////////////////////////////////////////////////////////////////////////////////////////////////////

/// Raw counters accumulated over a simulation run.
///
/// No derived rates or ratios are exposed here - callers compute those from
/// these totals, since there's no one right way to divide them for every use case.
// DESIGN(fidelicura): This struct used to also carry `hit_rate`, `miss_rate`,
// `mpkb`, `energy_per_prediction`, `energy_per_operation`, `energy_per_hit`,
// `energy_per_area`, and other methods. However, they have been removed as the
// set of useful ratios over raw totals is unbounded (weighted, windowed,
// per-kind, ...), so any fixed list is arbitrary and basically endless.
#[derive(Debug, Default, Clone, Copy, PartialEq, PartialOrd)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Metrics {
    /// Number of branches scored after warmup.
    pub total_predictions: u64,
    /// Number of metered operations executed.
    pub total_operations: u64,
    /// Number of correct early next-PC predictions (direction and next address).
    pub total_hits: u64,
    /// Number of branches whose direction was scored.
    pub direction_predictions: u64,
    /// Number of branches with the correct direction.
    pub direction_hits: u64,
    /// Number of branches whose next address was scored.
    pub target_predictions: u64,
    /// Number of branches with the correct predicted next address.
    pub target_hits: u64,
    /// Total switching energy charged, summed every time an operation runs.
    pub total_energy: f64,
    /// Total static space area of the distinct operations, summed once at usage of an operation.
    pub total_space: f64,
    /// Total leakage energy accrued, ticked once per cycle for every distinct operation.
    pub total_leakage: f64,
    /// Total modeled work time: serial operations sum; operations in one `overlap!` use their maximum.
    pub total_time: u64,
}

impl Metrics {
    /// Takes this thread's metrics, leaving a fresh run's worth
    /// of [`Ledger`] state - including space-charging - behind.
    pub(crate) fn reset() -> Self {
        LEDGER.with(|cell| cell.replace(Ledger::new())).metrics
    }

    /// Run `f` as one overlap group: operation work times combine by maximum.
    /// Nested groups contribute their maximum latency as one operation to their parent.
    #[doc(hidden)] // Used by the exported `overlap!` macro.
    pub fn overlap<R>(f: impl FnOnce() -> R) -> R {
        LEDGER.with(|cell| {
            let mut ledger = cell.borrow_mut();
            ledger.overlaps.push(0);
        });

        let guard = overlap::Guard;
        let result = f();
        drop(guard);
        result
    }

    /// Enable or disable metering for subsequent operations this run.
    ///
    /// Used by the simulator to suppress charging during an unmetered warmup window.
    pub(crate) fn turn(on: bool) {
        LEDGER.with(|cell| cell.borrow_mut().charging = on);
    }

    /// Tick one cycle of leakage; call once per cycle.
    ///
    /// Adds every instantiated operation's `leakage_energy` (see
    /// [`Metrics::charge`]) into [`Metrics::leakage`], gated by
    /// the same metering flag as [`Metrics::charge`].
    pub(crate) fn tick() {
        LEDGER.with(|cell| {
            let mut ledger = cell.borrow_mut();
            if ledger.charging {
                ledger.metrics.total_leakage += ledger.leakage;
            }
        });
    }

    /// Score one branch's direction and next address; `hits` records when both are correct.
    pub(crate) fn prediction(direction_hit: bool, target_hit: bool) {
        LEDGER.with(|cell| {
            let mut ledger = cell.borrow_mut();
            ledger.metrics.total_predictions += 1;
            ledger.metrics.direction_predictions += 1;
            ledger.metrics.target_predictions += 1;
            direction_hit.then(|| ledger.metrics.direction_hits += 1);
            target_hit.then(|| ledger.metrics.target_hits += 1);
            (direction_hit && target_hit).then(|| ledger.metrics.total_hits += 1);
        });
    }

    /// Charge an [`Operation`] its full [`Cost`].
    ///
    /// Panics if any `Cost` field was left unset - that surfaces a mispriced model.
    ///
    /// `switching_energy` and `latency_ticks` accrue every time (plus one counted
    /// operation), while `area_space` and `leakage_energy` are added only the
    /// first time this exact `(operation, SIZE)` pair is charged this run, since
    /// that hardware exists (and leaks) once instantiated however often it then
    /// executes. `SIZE` distinguishes different instances of the same operation's
    /// hardware (e.g. `BITS` width, or a multiplexer's `ways`) - reusing the same
    /// `SIZE` is the same hardware reused, a different `SIZE` is a separate
    /// element. A const generic, since every caller knows its size at compile time.
    pub(crate) fn charge<const SIZE: usize>(operation: Operation, cost: Cost) {
        let energy = cost.switching_energy.unwrap_or_else(|| {
            panic!("`Costs::{operation:?}` charged but `Cost::switching_energy` left unset")
        });
        let time = cost.latency_ticks.unwrap_or_else(|| {
            panic!("`Costs::{operation:?}` charged but `Cost::latency_ticks` left unset")
        });
        let space = cost.area_space.unwrap_or_else(|| {
            panic!("`Costs::{operation:?}` charged but `Cost::area_space` left unset")
        });
        let leakage = cost.leakage_energy.unwrap_or_else(|| {
            panic!("`Costs::{operation:?}` charged but `Cost::leakage_energy` left unset")
        });

        LEDGER.with(|cell| {
            let mut ledger = cell.borrow_mut();
            if !ledger.charging {
                return;
            }

            ledger.metrics.total_energy += energy;
            if let Some(overlap) = ledger.overlaps.last_mut() {
                *overlap = (*overlap).max(time);
            } else {
                ledger.metrics.total_time += time;
            }
            ledger.metrics.total_operations += 1;

            if ledger.charged.insert((operation, SIZE)) {
                ledger.metrics.total_space += space;
                ledger.leakage += leakage;
            }
        });
    }

    /// Charge a predictor's state area, [`Predictor::size`], straight into `total_space`.
    ///
    /// Area only - state holds bits, it is not executed, so it has no
    /// per-execution energy. Call once per run. A stateless predictor reports
    /// `0.0` and adds nothing.
    pub(crate) fn storage<const BITS: usize, P>(predictor: &P)
    where
        P: Predictor<BITS>,
    {
        LEDGER.with(|cell| cell.borrow_mut().metrics.total_space += predictor.size());
    }

    /// Charge a `WAYS`-way multiplexer, modelled as `WAYS - 1` input selects.
    ///
    /// Its energy, area, and leakage all scale with `WAYS - 1`; a 0- or 1-way
    /// select is free. `WAYS` is also the [`Metrics::charge`] size key, so a
    /// 2-way and an 8-way `select!` are priced as the two separate multiplexers
    /// they are, each charged once.
    #[doc(hidden)] // XXX(fidelicura): Public only so the `select!` macro can expand at call sites.
    pub fn select<const WAYS: usize>() {
        let cost = COSTS.with(Cell::get).select;
        let scale = const { WAYS.saturating_sub(1) };

        Self::charge::<WAYS>(
            Operation::select,
            Cost {
                switching_energy: cost.switching_energy.map(|energy| energy * scale as f64),
                leakage_energy: cost.leakage_energy.map(|leakage| leakage * scale as f64),
                area_space: cost.area_space.map(|space| space * scale as f64),
                latency_ticks: cost.latency_ticks.map(|time| time * scale as u64),
            },
        );
    }

    pub fn push(latency_delay: u64) {
        LEDGER.with(|cell| {
            let mut ledger = cell.borrow_mut();
            if !ledger.charging {
                return;
            }

            if let Some(overlap) = ledger.overlaps.last_mut() {
                *overlap = (*overlap).max(latency_delay);
            } else {
                ledger.metrics.total_time += latency_delay;
            }
        });
    }
}

////////////////////////////////////////////////////////////////////////////////////////////////////
// }} Metrics
////////////////////////////////////////////////////////////////////////////////////////////////////

mod overlap {
    use super::*;

    pub struct Guard;

    impl Drop for Guard {
        fn drop(&mut self) {
            LEDGER.with(|cell| {
                let mut ledger = cell.borrow_mut();
                let time = ledger
                    .overlaps
                    .pop()
                    .expect("overlap guard without active overlap group");
                if !ledger.charging {
                    return;
                }

                if let Some(parent) = ledger.overlaps.last_mut() {
                    *parent = (*parent).max(time);
                } else {
                    ledger.metrics.total_time += time;
                }
            });
        }
    }

    /// Execute a block whose metered operations are modeled as overlapping.
    ///
    /// Rust evaluates the block normally and sequentially; the macro does not
    /// create threads or infer dependencies. The caller asserts that the
    /// metered operations can overlap: their work times contribute the maximum
    /// latency, while energy and operation counts still sum. Area remains
    /// charged once per distinct operation, and leakage continues to accrue on
    /// simulator ticks. Nested groups contribute their maximum latency as one
    /// operation to the surrounding group.
    ///
    /// Write independent operations as ordinary statements:
    ///
    /// ```ignore
    /// overlap! {
    ///     buffer[index] = target;
    ///     valid[index] = Value::new(1);
    /// }
    /// ```
    #[macro_export]
    macro_rules! overlap {
        ($($body:tt)*) => {{ $crate::Metrics::overlap(|| { $($body)* }) }};
    }
}

////////////////////////////////////////////////////////////////////////////////////////////////////
// Costs {{
////////////////////////////////////////////////////////////////////////////////////////////////////

thread_local! {
    /// Per-thread cost table consulted by every charged operation.
    ///
    /// Thread-locality keeps concurrent simulation runs and tests isolated.
    static COSTS: Cell<Costs> = Cell::new(Costs::default());
}

/// Cost of one operation: dynamic `switching_energy` and `latency_ticks`, and
/// static `area_space` and `leakage_energy` ticked every cycle once the
/// operation's hardware exists.
///
/// Any field left `None` means the operation is not expected to run, so charging
/// it panics - that surfaces a mispriced model. Price an operation at zero
/// explicitly with [`Cost::free`]. To model several instances of an operation,
/// set `area_space` to their combined area and `leakage_energy` to their combined drain.
#[derive(Debug, Default, Clone, Copy, PartialEq, PartialOrd)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Cost {
    pub switching_energy: Option<f64>,
    pub leakage_energy: Option<f64>,
    pub area_space: Option<f64>,
    pub latency_ticks: Option<u64>,
}

impl Cost {
    /// Zero energy, space, leakage, and time; nothing panics, nothing accrues.
    pub const fn free() -> Self {
        Self {
            switching_energy: Some(0.0),
            leakage_energy: Some(0.0),
            area_space: Some(0.0),
            latency_ticks: Some(0),
        }
    }
}

impl Costs {
    /// Replace this thread's cost table.
    pub(crate) fn set(value: Self) {
        COSTS.with(|cell| cell.set(value))
    }

    /// Copy of this thread's cost table.
    pub(crate) fn get() -> Self {
        COSTS.with(|cell| cell.get())
    }
}

/// Declares the [`Costs`] table from one list of operation names.
macro_rules! costs {
    ($($field:ident),+ $(,)?) => {
        /// Operation identity.
        ///
        /// One variant per cost field; paired with a `size` (see
        /// [`Metrics::charge`]) it keys which distinct hardware
        /// instances have already had their static space counted this run.
        #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
        #[allow(non_camel_case_types)] // Variants mirror the `Value` method names.
        pub(crate) enum Operation {
            $($field),+
        }

        /// Per-operation cost table.
        ///
        /// Each field is named exactly after the `Value` method
        /// it prices; see [`Cost`] for the per-field semantics.
        #[derive(Debug, Default, Clone, Copy, PartialEq, PartialOrd)]
        #[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
        #[cfg_attr(feature = "serde", serde(default))]
        pub struct Costs {
            $(pub $field: Cost),+
        }

        impl Costs {
            /// A cost table with every operation [`Cost::free`].
            pub fn free() -> Self {
                Self { $($field: Cost::free()),+ }
            }
        }
    };
}

costs! {
    checked_add, checked_div, checked_mul, checked_neg, checked_pow,
    checked_rem, checked_shl, checked_shr, checked_sub,

    overflowing_add, overflowing_div, overflowing_mul, overflowing_neg, overflowing_pow,
    overflowing_rem, overflowing_shl, overflowing_shr, overflowing_sub,

    saturating_add, saturating_div, saturating_mul, saturating_neg, saturating_pow,
    saturating_sub,

    strict_add, strict_div, strict_mul, strict_neg, strict_pow,
    strict_rem, strict_shl, strict_shr, strict_sub,

    wrapping_add, wrapping_div, wrapping_mul, wrapping_neg, wrapping_pow,
    wrapping_rem, wrapping_shl, wrapping_shr, wrapping_sub,

    bitand, bitor, bitxor, bitnot,

    memory_read, memory_write,
    register_read, register_write,

    select, assign, resize,
}

////////////////////////////////////////////////////////////////////////////////////////////////////
// }} Costs
////////////////////////////////////////////////////////////////////////////////////////////////////

#[cfg(test)]
mod tests {
    use super::*;

    // XXX(fidelicura): State lives in thread-locals shared by every test on the
    // same thread, so each test starts with `Metrics::reset` to avoid cross-test
    // clashing and restores the default cost table when it touches it.

    #[test]
    fn prediction_tracks_independent_scores() {
        Metrics::reset();
        Metrics::prediction(true, true);
        Metrics::prediction(false, false);
        Metrics::prediction(true, false);

        let m = Metrics::reset();
        assert_eq!(m.total_predictions, 3);
        assert_eq!((m.direction_predictions, m.direction_hits), (3, 2));
        assert_eq!((m.target_predictions, m.target_hits), (3, 1));
        assert_eq!(m.total_hits, 1);
    }

    /// Price `wrapping_add` at `cost`, charge it `n` times, return the run's metrics.
    fn charge_n(cost: Cost, n: usize) -> Metrics {
        Costs::set(Costs {
            wrapping_add: cost,
            ..Costs::default()
        });
        Metrics::reset();
        for _ in 0..n {
            Metrics::charge::<0>(Operation::wrapping_add, Costs::get().wrapping_add);
        }
        Metrics::reset()
    }

    #[test]
    fn reset_returns_accumulated_then_clears() {
        let cost = Cost {
            switching_energy: Some(5.0),
            ..Cost::free()
        };
        assert_eq!(charge_n(cost, 1).total_energy, 5.0); // Hands back the total...
        assert_eq!(Metrics::reset(), Metrics::default()); // ...and leaves it cleared.
    }

    #[test]
    fn charge_accumulates_energy_every_time() {
        let cost = Cost {
            switching_energy: Some(1.5),
            ..Cost::free()
        };
        assert_eq!(charge_n(cost, 2).total_energy, 3.0);
    }

    #[test]
    fn charge_counts_space_once_however_often_charged() {
        let cost = Cost {
            switching_energy: Some(1.0),
            area_space: Some(2.0),
            ..Cost::free()
        };
        let m = charge_n(cost, 4);
        assert_eq!(m.total_energy, 4.0); // Energy is per execution.
        assert_eq!(m.total_space, 2.0); // Area counted once, however often charged.
    }

    #[test]
    fn tick_adds_leakage_once_per_cycle_for_instantiated_operations() {
        let cost = Cost {
            leakage_energy: Some(1.5),
            ..Cost::free()
        };
        Costs::set(Costs {
            wrapping_add: cost,
            ..Costs::default()
        });
        Metrics::reset();

        // Charging the same operation twice instantiates its leakage once...
        Metrics::charge::<0>(Operation::wrapping_add, cost);
        Metrics::charge::<0>(Operation::wrapping_add, cost);

        // ...but every tick adds that leakage again, since it drains continuously.
        Metrics::tick();
        Metrics::tick();

        let m = Metrics::reset();
        assert_eq!(m.total_leakage, 3.0); // 1.5 leakage * 2 ticks.

        Costs::set(Costs::default());
    }

    #[test]
    fn tick_suppressed_while_metering_off() {
        let cost = Cost {
            leakage_energy: Some(9.0),
            ..Cost::free()
        };
        Costs::set(Costs {
            wrapping_add: cost,
            ..Costs::default()
        });
        Metrics::reset();

        Metrics::charge::<0>(Operation::wrapping_add, cost);
        Metrics::turn(false); // As during an unmetered warmup window.
        Metrics::tick(); // Dropped, not accrued.

        let m = Metrics::reset(); // `reset` restores metering.
        assert_eq!(m.total_leakage, 0.0);

        Costs::set(Costs::default());
    }

    #[test]
    fn select_scales_energy_space_and_leakage_by_extra_inputs() {
        Costs::set(Costs {
            select: Cost {
                switching_energy: Some(10.0),
                leakage_energy: Some(3.0),
                area_space: Some(4.0),
                latency_ticks: Some(2),
            },
            ..Costs::default()
        });

        Metrics::reset();
        Metrics::select::<4>(); // 4 inputs == 3 selects.
        Metrics::tick();
        let m = Metrics::reset();
        assert_eq!(m.total_energy, 30.0); // 10 * 3.
        assert_eq!(m.total_space, 12.0); // 4 * 3.
        assert_eq!(m.total_leakage, 9.0); // 3 * 3, ticked once.
        assert_eq!(m.total_time, 6); // 2 * 3.

        Metrics::select::<1>(); // Single input is free.
        assert_eq!(Metrics::reset().total_energy, 0.0);

        Metrics::select::<0>(); // Saturating: no underflow.
        assert_eq!(Metrics::reset().total_energy, 0.0);

        Costs::set(Costs::default());
    }

    #[test]
    fn costs_get_returns_what_set_stored() {
        let costs = Costs {
            wrapping_add: Cost {
                switching_energy: Some(7.0),
                area_space: Some(1.0),
                ..Cost::free()
            },
            ..Costs::default()
        };
        Costs::set(costs);

        assert_eq!(Costs::get(), costs);

        Costs::set(Costs::default());
    }

    #[test]
    #[should_panic(
        expected = "`Costs::wrapping_add` charged but `Cost::switching_energy` left unset"
    )]
    fn charge_panics_on_unset_energy() {
        Metrics::charge::<0>(Operation::wrapping_add, Costs::default().wrapping_add);
    }

    #[test]
    #[should_panic(expected = "`Costs::wrapping_add` charged but `Cost::latency_ticks` left unset")]
    fn charge_panics_on_unset_time() {
        let costs = Costs {
            wrapping_add: Cost {
                switching_energy: Some(1.0),
                ..Cost::default()
            },
            ..Costs::default()
        };
        Metrics::charge::<0>(Operation::wrapping_add, costs.wrapping_add);
    }

    #[test]
    #[should_panic(expected = "`Costs::wrapping_add` charged but `Cost::area_space` left unset")]
    fn charge_panics_on_unset_space() {
        let costs = Costs {
            wrapping_add: Cost {
                switching_energy: Some(1.0),
                leakage_energy: Some(1.0),
                area_space: None,
                latency_ticks: Some(1),
            },
            ..Costs::default()
        };
        Metrics::charge::<0>(Operation::wrapping_add, costs.wrapping_add);
    }

    #[test]
    #[should_panic(
        expected = "`Costs::wrapping_add` charged but `Cost::leakage_energy` left unset"
    )]
    fn charge_panics_on_unset_leakage() {
        let costs = Costs {
            wrapping_add: Cost {
                switching_energy: Some(1.0),
                leakage_energy: None,
                area_space: Some(1.0),
                latency_ticks: Some(1),
            },
            ..Costs::default()
        };
        Metrics::charge::<0>(Operation::wrapping_add, costs.wrapping_add);
    }

    #[test]
    fn free_charges_no_energy_space_or_leakage() {
        let m = charge_n(Cost::free(), 3);
        assert_eq!(m.total_energy, 0.0);
        assert_eq!(m.total_space, 0.0);
        assert_eq!(m.total_leakage, 0.0);
        assert_eq!(m.total_time, 0);
    }

    #[test]
    fn nested_overlap_groups_contribute_their_maximum_to_parent() {
        Metrics::reset();
        Metrics::overlap(|| {
            Metrics::push(3);
            Metrics::overlap(|| {
                Metrics::push(5);
                Metrics::push(7);
            });
            Metrics::push(4);
        });
        Metrics::push(2);

        assert_eq!(Metrics::reset().total_time, 9);
    }

    #[test]
    fn charge_accumulates_latency_ticks_every_time() {
        let cost = Cost {
            latency_ticks: Some(2),
            ..Cost::free()
        };
        assert_eq!(charge_n(cost, 4).total_time, 8);
    }

    #[test]
    fn charge_counts_space_once_per_distinct_size_not_per_operation() {
        let cost = Cost {
            area_space: Some(2.0),
            ..Cost::free()
        };
        Costs::set(Costs {
            wrapping_add: cost,
            ..Costs::default()
        });
        Metrics::reset();

        // Same size charged twice: one hardware instance, area counted once.
        Metrics::charge::<8>(Operation::wrapping_add, cost);
        Metrics::charge::<8>(Operation::wrapping_add, cost);
        assert_eq!(Metrics::reset().total_space, 2.0);

        // Different sizes of the same operation: two distinct instances, both counted.
        Metrics::charge::<8>(Operation::wrapping_add, cost);
        Metrics::charge::<64>(Operation::wrapping_add, cost);
        assert_eq!(Metrics::reset().total_space, 4.0); // 2.0 (8-bit) + 2.0 (64-bit).

        Costs::set(Costs::default());
    }

    #[test]
    fn select_of_different_ways_each_charge_their_own_area() {
        Costs::set(Costs {
            select: Cost {
                switching_energy: Some(1.0),
                leakage_energy: Some(1.0),
                area_space: Some(4.0),
                latency_ticks: Some(1),
            },
            ..Costs::default()
        });
        Metrics::reset();

        Metrics::select::<2>(); // 2-way mux: first time, area counted (1 select * 4.0).
        Metrics::select::<2>(); // Same width again: no extra area.
        Metrics::select::<8>(); // Different width: a distinct mux, area counted too (7 selects * 4.0).

        let m = Metrics::reset();
        assert_eq!(m.total_space, 32.0); // 4.0 (2-way, scale 1) + 28.0 (8-way, scale 7), not 4.0 total.

        Costs::set(Costs::default());
    }
}
