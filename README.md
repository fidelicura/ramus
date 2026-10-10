<!--
Copyright 2026 Ramus

Licensed under the Apache License, Version 2.0 (the "License");
you may not use this file except in compliance with the License.
You may obtain a copy of the License at

    http://www.apache.org/licenses/LICENSE-2.0

Unless required by applicable law or agreed to in writing, software
distributed under the License is distributed on an "AS IS" BASIS,
WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
See the License for the specific language governing permissions and
limitations under the License.
-->

<div align="center">

[Overview] • [Example] • [Documentation] • [Contribute] • [License]

<h3>A trace-driven branch-predictor evaluation framework for simulation.</h3>

```toml
[dependencies]
ramus = "0.0.0"
```

</div>

## Overview

[Ramus] runs a branch predictor over a trace of branch records and tells you two
things: how well it predicted, and what it would cost in hardware. You write the
predictor in ordinary Rust. The framework drives it over the trace, scores it,
and adds up the energy and area of every operation it does along the way.

You, as a user of the framework, get (**1**) predictions, hits, misses and hit
rate metrics over a full trace, (**2**) dynamic energy and static area metrics,
accumulated per operation, taken straight from the arithmetic and data movements
your predictor performs, (**3**) fully tested and 100% line and branch coverage
API without any unsafe code, (**4**) support for any trace format behind one
small trait, (**5**) feature-flagged parallel implementation of sweeps over
many predictors/traces.

## Prediction

`Predictor::predict(current)` returns one `Prediction<BITS>` containing
both `direction` and `target`. Every branch kind is predicted, scored, and passed
to `update`; warmup and speculation apply to every branch as well. Only the
address is supplied to `predict`; branch kind is revealed during `update`.

A report separates direction accuracy, next-address accuracy, and early next-PC
accuracy. Both direction and next address are scored for every CFI, regardless
of kind or whether it is taken. `Info::next` is the actual next address: the
target when taken, or fall-through when untaken. A cold predictor must still
provide a best-effort next address; the example below falls back to sequential
`PC + 4`. Scoring itself adds no hardware cost. Reports count one branch
prediction per CFI after warmup.

Arbiter's `--predictors smith_bimodal` combines 1024 two-bit direction counters
with a tagged, direct-mapped target buffer. Direction comes from
the counters, which train on every branch kind. Taken branches train the target
buffer. All tables contribute to storage and operation costs.

## Example

> See more examples in [`arbiter`].

```rust
////////////////////////////////////////////////////////////////////////////////
// 0. BACKGROUND
//
// We're building a Smith's 2-bit predictor: the classic "remember, per branch,
// which way it usually goes", which is also called "2-bit saturating counter".
// A table of small saturating counters, one bucket per branch address. Read
// the bucket to predict; nudge it afterwards toward what actually happened.
// A tagged buffer remembers each taken target, so every prediction supplies
// both the direction and the destination.
//
// The one rule to internalise: you model hardware with `Value` and `Array`,
// not plain integers. Every arithmetic operation on a `Value` is metered,
// so its energy and area land in the final report automatically.
//
// Three parts below: the predictor itself (1), a tiny trace to feed it (2),
// and the simulator run that scores both (3).
////////////////////////////////////////////////////////////////////////////////

////////////////////////////////////////////////////////////////////////////////
// 1. PREDICTOR {{
////////////////////////////////////////////////////////////////////////////////

use ramus::{Address as BranchAddress, Target, Kind, Prediction, Array, Direction, Predictor, Value, select};

/// Bimodal direction counters plus a tagged, direct-mapped target buffer.
#[derive(Default)]
struct Bimodal {
    table: Array<2, { Self::LENGTH }>,
    tags: Array<64, { Self::LENGTH }>,
    targets: Array<64, { Self::LENGTH }>,
    valid: Array<1, { Self::LENGTH }>,
}

impl Bimodal {
    /// Number of buckets in the table - one per branch address (after folding).
    ///
    /// Kept a power of two so the index can use a cheap AND mask (see `MASK`).
    const LENGTH: usize = 2usize.pow(10);

    /// A power-of-two table size lets us fold an address into a bucket with a
    /// single AND instead of a `%`: `addr & (LENGTH - 1)` is the bottom 10 bits.
    const MASK: u32 = Self::LENGTH as u32 - 1;

    /// Map a branch address to its bucket index.
    fn index(address: Address) -> Value<64> {
        // Instructions are 4-byte aligned, so the low 2 bits are always zero
        // and carry no information. Drop them first so neighbouring branches
        // don't all collide into the same bucket.
        let folded = address >> 2u8;
        // Keep only the low 10 bits => an index in 0..1024.
        folded & Self::MASK
    }
}

impl Predictor<64> for Bimodal {
    fn predict(&mut self, address: Address) -> Prediction<64> {
        let index = Self::index(address);
        let counter = self.table[index];
        // Top bit of the 2-bit counter is the verdict: 2 or 3 => "taken".
        let taken = (counter >> 1u8) == 1u8;
        let hit = (self.valid[index] == 1u8) & (self.tags[index] == address);
        let buffered_target: Target<64> = self.targets[index];
        let target = select!(hit => {
            true => buffered_target,
            false => address.wrapping_add(4u8),
        });
        Prediction { direction: Direction::from(taken), target }
    }

    fn update(&mut self, current: Address, _kind: Kind, direction: Direction, next: Address) {
        let index = Self::index(current);
        // Learn direction for every branch kind, including unconditional branches.
        let counter = self.table[index];
        self.table[index] = select!(direction => {
            Direction::Taken => {
                self.tags[index] = current;
                self.targets[index] = next;
                self.valid[index] = Value::new(1);
                counter.saturating_add(1u8)
            },
            Direction::Untaken => counter.saturating_sub(1u8),
        });
    }

    fn size(&self) -> f64 {
        Array::<2, { Self::LENGTH }>::SIZE
            + 2.0 * Array::<64, { Self::LENGTH }>::SIZE
            + Array::<1, { Self::LENGTH }>::SIZE
    }
}

type Address = BranchAddress<64>;

////////////////////////////////////////////////////////////////////////////////
// }} 1. PREDICTOR
////////////////////////////////////////////////////////////////////////////////

////////////////////////////////////////////////////////////////////////////////
// 2. WIRING {{
//
// A predictor runs over a `Source`: any iterator of branch records. Implement
// the one `fetch` method and any trace format works - Gzip, binary, in-memory.
// Here is a small one just for the example.
////////////////////////////////////////////////////////////////////////////////

use ramus::{Info, Source};
use std::collections::VecDeque;

// Hands out pre-built records front-to-back, then `None` once drained.
struct Trace(VecDeque<Info<64>>);

impl Source<64> for Trace {
    fn fetch(&mut self) -> Option<Info<64>> {
        self.0.pop_front()
    }
}

impl Trace {
    // A tiny imaginary trace: two conditional branches, one mostly taken
    // and one mostly not, so the counters have something to learn from.
    fn example() -> Self {
        let branch = |address: u64, direction: Direction, next: u64| Info::<64> {
            address: Value::wrapping(address), // Branch instruction PC.
            direction,
            kind: Kind::Conditional,
            next: Value::wrapping(next), // Branch resolution PC.
        };

        Self(VecDeque::from([
            branch(0x400, Direction::Taken, 0x480),
            branch(0x400, Direction::Taken, 0x480),
            branch(0x420, Direction::Untaken, 0x424),
            branch(0x400, Direction::Untaken, 0x404),
            branch(0x420, Direction::Untaken, 0x424),
        ]))
    }
}

////////////////////////////////////////////////////////////////////////////////
// }} 2. WIRING
////////////////////////////////////////////////////////////////////////////////

////////////////////////////////////////////////////////////////////////////////
// 3. DRIVING {{
//
// Hand the `Simulator` a `Costs` table that fully prices every operation the
// predictor performs, then it drives the predictor over the trace and returns
// `Metrics`. Each price has four parts: `switching_energy` and `latency_ticks`
// accrue on every execution; `area_space` is the operation's area, counted
// once per distinct size that operation runs at this run (e.g., once for
// `Value<8>::wrapping_add`, again for `Value<64>::wrapping_add` - different
// widths are different hardware); `leakage_energy` is that same hardware's
// static drain, ticked once per cycle (once per fetched record) for as long
// as it exists this run. To model several instances of a component at the
// same size, set its `area_space` to their combined area and `leakage_energy`
// to their combined drain. `latency_ticks` sums like energy into `Metrics` - the
// framework has no dependency graph between operations, so it does not compute
// a critical path, only the total work time charged over the run.
////////////////////////////////////////////////////////////////////////////////

use ramus::{Config, Cost, Costs, Simulator};

fn main() {
    // XXX: `switching_energy`, `leakage_energy`, `area_space`, AND `latency_ticks`
    // must be set on any operation that actually runs. Leave any at `None` and
    // the framework PANICS the moment that operation is charged. This is
    // deliberate: every hardware element has to be measurable, so a hole in
    // your cost model fails loudly instead of silently reporting a partial
    // number that misinforms you about the design's true costs.
    //
    // HINT: `serde` feature flag derives `serde::{Deserialize, Serialize}` on
    // data-containing structures, so results dump straight to format you need.
    let costs = Costs {
        wrapping_shr:   Cost { switching_energy: Some(0.5), leakage_energy: Some(0.05), area_space: Some(0.6), latency_ticks: Some(2) }, // `>>`: index/counter shifters.
        wrapping_add:   Cost { switching_energy: Some(0.3), leakage_energy: Some(0.03), area_space: Some(0.4), latency_ticks: Some(2) }, // Sequential PC fallback on a target-buffer miss.
        bitand:         Cost { switching_energy: Some(0.1), leakage_energy: Some(0.01), area_space: Some(0.2), latency_ticks: Some(1) }, // `&`: mask AND over the index.
        saturating_add: Cost { switching_energy: Some(0.3), leakage_energy: Some(0.03), area_space: Some(0.4), latency_ticks: Some(2) }, // `Value::saturating_add`: incrementer.
        saturating_sub: Cost { switching_energy: Some(0.3), leakage_energy: Some(0.03), area_space: Some(0.4), latency_ticks: Some(2) }, // `Value::saturating_sub`: decrementer.
        select:         Cost { switching_energy: Some(0.2), leakage_energy: Some(0.02), area_space: Some(0.3), latency_ticks: Some(1) }, // `select!`: 2-input multiplexer.
        memory_read:    Cost { switching_energy: Some(0.4), leakage_energy: Some(0.04), area_space: Some(0.5), latency_ticks: Some(3) }, // `Array[i]`: table read port.
        memory_write:   Cost { switching_energy: Some(0.5), leakage_energy: Some(0.05), area_space: Some(0.5), latency_ticks: Some(3) }, // `Array[i] = _`: table write port.
        ..Default::default()
    };
    let source = Trace::example();
    let predictor = Bimodal::default();
    // `Config::none` scores from the first branch and resolves every prediction
    // immediately; set `warmup` to train over an unmetered leading window before
    // scoring begins, or `speculations` to let up to that
    // many predictions sit unresolved before training lags behind them.
    let config = Config::none();

    // HINT: `Simulator::run_many` runs every source, config, and predictor
    // combination; with the `parallel` feature, it uses a bounded thread pool.
    //
    let metrics = Simulator::run_one(source, predictor, costs, config);
    println!("predictions:  {:.3}", metrics.total_predictions); // Total amount of predictions performed over trace.
    println!("hits:         {:.3}", metrics.total_hits);        // Total amount of correct predictions performed over trace.
    println!("energy:       {:.1}", metrics.total_energy);      // Total switching energy costs in conventional units.
    println!("leakage:      {:.1}", metrics.total_leakage);     // Total leakage energy costs ticked over the run's cycles.
    println!("area:         {:.1}", metrics.total_space);       // Total space costs of the storage and execution units.
    println!("time:         {:.1}", metrics.total_time);        // Total work time charged over every metered execution.

    assert_eq!(metrics.total_predictions, 5);
    assert_eq!(metrics.total_hits, 2);
}

////////////////////////////////////////////////////////////////////////////////
// }} 3. DRIVING
////////////////////////////////////////////////////////////////////////////////
```

<!-- Links -->

[Overview]: #overview
[Example]: #example
[Documentation]: https://docs.rs/ramus
[Contribute]: ./CONTRIBUTING.md
[License]: ./LICENSE
[Ramus]: https://github.com/fidelicura/ramus
[`arbiter`]: ./arbiter/src/predictors
