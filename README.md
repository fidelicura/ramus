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

use ramus::{Address, Array, Outcome, Predictor, Value, select};

// The whole predictor is one table of 2-bit saturating counters - one bucket
// per branch address. `Array<BITS, LENGTH>` is the framework's hardware-aware
// array: `LENGTH` cells, each a `Value<BITS>` (an unsigned integer modelled at
// exactly `BITS` wide, so its width shows up in the area report). Here that's
// 1024 buckets of `Value<2>` holding 0..=3, the four states of the classic
// Smith predictor: strongly/weakly untaken (0, 1) and weakly/strongly
// taken (2, 3).
//
// Every cell of an `Array` starts at zero, so its `Default` gives us a fully
// initialized table for free - that's why we can `#[derive(Default)]` on the
// whole predictor instead of hand-writing a constructor that zeroes 1024 cells.
#[derive(Default)]
struct Bimodal {
    table: Array<2, { Self::LENGTH }>,
}

impl Bimodal {
    // Number of buckets in the table - one per branch address (after folding).
    // Kept a power of two so the index can use a cheap AND mask (see `MASK`).
    const LENGTH: usize = 1024;

    // A power-of-two table size lets us fold an address into a bucket with a
    // single AND instead of a `%`: `addr & (LENGTH - 1)` is the bottom 10 bits.
    const MASK: u32 = Self::LENGTH as u32 - 1;

    // Map a branch address to its bucket index.
    fn index(address: Address) -> Value<64> {
        // Instructions are 4-byte aligned, so the low 2 bits are always zero
        // and carry no information. Drop them first so neighbouring branches
        // don't all collide into the same bucket.

        // Note that `>>` on a `Value` is metered by space and energy costs,
        // as well as all other operations on `Value` further down.
        let folded = address >> 2u8;

        // Keep only the low 10 bits => an index in 0..1024.
        folded & Self::MASK
    }
}

// A predictor is just these two methods (there's also an optional `track` for
// history-based designs; the default no-op is fine for bimodal).
impl Predictor for Bimodal {
    // This trait method is called once per conditional branch, before we learn the real outcome.
    fn predict(&mut self, current: Address) -> Outcome {
        let index = Self::index(current);
        let counter = self.table[index]; // Indexing is also metered.

        // Top bit of the 2-bit counter is the verdict: taken or not.
        let taken = (counter >> 1u8) == 1u8;
        Outcome::from(taken)
    }

    // Called right after, with the branch's true `outcome` (`next` is the
    // resulting PC, handy for history-based predictors - unused here).
    fn update(&mut self, current: Address, outcome: Outcome, _next: Address) {
        let index = Self::index(current);
        let counter = self.table[index]; // Indexing is also metered.

        // Saturating, so the counter sticks at the extremes (3 stays 3): that
        // hysteresis is what stops one stray outcome from flipping the verdict.
        // `select!` is `match` that also charges a 2-input multiplexer cost.
        self.table[index] = select!(outcome => {
            Outcome::Taken   => counter.saturating_add(1u8),
            Outcome::Untaken => counter.saturating_sub(1u8),
        });
    }

    // The predictor's static storage area: the whole counter table. `Array`'s
    // associated `SIZE` is `BITS * LENGTH`, already in the same area units as
    // the cost model, so the framework adds it into `Metrics::space` field.
    fn size(&self) -> f64 {
        Array::<2, { Self::LENGTH }>::SIZE
    }
}

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

use ramus::{Info, Kind, Source};
use std::collections::VecDeque;

// Hands out pre-built records front-to-back, then `None` once drained.
struct Trace(VecDeque<Info>);

impl Source for Trace {
    fn fetch(&mut self) -> Option<Info> {
        self.0.pop_front()
    }
}

impl Trace {
    // A tiny imaginary trace: two conditional branches, one mostly taken
    // and one mostly not, so the counters have something to learn from.
    fn example() -> Self {
        let branch = |address: u64, outcome: Outcome, next: u64| Info {
            address: Value::wrapping(address), // Branch instruction PC.
            outcome,
            kind: Kind::Conditional,
            next: Value::wrapping(next), // Branch resolution PC.
        };

        Self(VecDeque::from([
            branch(0x400, Outcome::Taken, 0x480),
            branch(0x400, Outcome::Taken, 0x480),
            branch(0x420, Outcome::Untaken, 0x424),
            branch(0x400, Outcome::Untaken, 0x404),
            branch(0x420, Outcome::Untaken, 0x424),
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
// `Metrics`. Each price has four parts: `switching_energy` and `work_time`
// accrue on every execution; `area_space` is the operation's area, counted
// once per distinct size that operation runs at this run (e.g., once for
// `Value<8>::wrapping_add`, again for `Value<64>::wrapping_add` - different
// widths are different hardware); `leakage_energy` is that same hardware's
// static drain, ticked once per cycle (once per fetched record) for as long
// as it exists this run. To model several instances of a component at the
// same size, set its `area_space` to their combined area and `leakage_energy`
// to their combined drain. `work_time` sums like energy into `Metrics` - the
// framework has no dependency graph between operations, so it does not compute
// a critical path, only the total work time charged over the run.
////////////////////////////////////////////////////////////////////////////////

use ramus::{Config, Cost, Costs, Simulator};

fn main() {
    // XXX: `switching_energy`, `leakage_energy`, `area_space`, AND `work_time`
    // must be set on any operation that actually runs. Leave any at `None` and
    // the framework PANICS the moment that operation is charged. This is
    // deliberate: every hardware element has to be measurable, so a hole in
    // your cost model fails loudly instead of silently reporting a partial
    // number that misinforms you about the design's true costs.
    //
    // HINT: `serde` feature flag derives `serde::{Deserialize, Serialize}` on
    // data-containing structures, so results dump straight to format you need.
    let costs = Costs {
        wrapping_shr:   Cost { switching_energy: Some(0.5), leakage_energy: Some(0.05), area_space: Some(0.6), work_time: Some(0.2) }, // `>>`: index/counter shifters.
        bitand:         Cost { switching_energy: Some(0.1), leakage_energy: Some(0.01), area_space: Some(0.2), work_time: Some(0.1) }, // `&`: mask AND over the index.
        saturating_add: Cost { switching_energy: Some(0.3), leakage_energy: Some(0.03), area_space: Some(0.4), work_time: Some(0.2) }, // `Value::saturating_add`: incrementer.
        saturating_sub: Cost { switching_energy: Some(0.3), leakage_energy: Some(0.03), area_space: Some(0.4), work_time: Some(0.2) }, // `Value::saturating_sub`: decrementer.
        select:         Cost { switching_energy: Some(0.2), leakage_energy: Some(0.02), area_space: Some(0.3), work_time: Some(0.1) }, // `select!`: 2-input multiplexer.
        memory_read:    Cost { switching_energy: Some(0.4), leakage_energy: Some(0.04), area_space: Some(0.5), work_time: Some(0.3) }, // `Array[i]`: table read port.
        memory_write:   Cost { switching_energy: Some(0.5), leakage_energy: Some(0.05), area_space: Some(0.5), work_time: Some(0.3) }, // `Array[i] = _`: table write port.
        ..Default::default()
    };
    let source = Trace::example();
    let predictor = Bimodal::default();
    // `Config::none` scores from the first branch and resolves every prediction
    // immediately; set `warmup`/`charge` to train over (and optionally meter) a
    // leading window before scoring begins, or `speculation` to let up to that
    // many predictions sit unresolved before training lags behind them.
    let config = Config::none();

    // HINT: To compare many predictors or many traces in one go,
    // reach for `Simulator::{run_many_predictors,run_many_sources}`.
    //
    // HINT: You can spread the `Simulator::run_many_*` sweeps across
    // threads using `parallel` feature flag, which will use `rayon`.
    let metrics = Simulator::run_one(source, predictor, costs, config);
    println!("predictions:  {:.3}", metrics.predictions); // Total amount of predictions performed over trace.
    println!("hits:         {:.3}", metrics.hits);        // Total Amount of correct predictions performed over trace.
    println!("energy:       {:.1}", metrics.energy);      // Total switching energy costs in conventional units.
    println!("leakage:      {:.1}", metrics.leakage);     // Total leakage energy costs ticked over the run's cycles.
    println!("area:         {:.1}", metrics.space);       // Total space costs of the storage and execution units.
    println!("time:         {:.1}", metrics.time);        // Total work time charged over every metered execution.

    // Running this example prints:
    //
    //   predictions: 5
    //   hits:        2
    //   energy:      18.5
    //   leakage:     2.6
    //   area:        2051.5
    //   time:        10.5
    //
    // The counters start cold (strongly untaken), so only 2 of the 5 branches
    // are guessed right; the area is dominated by the 2048-bit counter table.
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
