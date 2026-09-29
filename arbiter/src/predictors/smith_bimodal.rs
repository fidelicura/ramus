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

use crate::predictors::Address;
use ramus::{Kind, Prediction, Array, Direction, Predictor, Value, select};

/// Bimodal direction counters plus a tagged, direct-mapped target buffer.
#[derive(Default)]
pub struct SmithBimodal {
    table: Array<2, { Self::LENGTH }>,
    tags: Array<64, { Self::LENGTH }>,
    targets: Array<64, { Self::LENGTH }>,
    valid: Array<1, { Self::LENGTH }>,
}

impl SmithBimodal {
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

impl Predictor<64> for SmithBimodal {
    fn predict(&mut self, current: Address, kind: Kind) -> Prediction<64> {
        let index = Self::index(current);
        let counter = self.table[index];
        // Top bit of the 2-bit counter is the verdict: 2 or 3 => "taken".
        let taken = kind != Kind::Conditional || (counter >> 1u8) == 1u8;
        let hit = (self.valid[index] == 1u8) & (self.tags[index] == current);
        let target = self.targets[index];
        Prediction { direction: Direction::from(taken), target: hit.then_some(target) }
    }

    fn update(&mut self, current: Address, kind: Kind, direction: Direction, next: Address) {
        let index = Self::index(current);
        if kind == Kind::Conditional {
            let counter = self.table[index];
            self.table[index] = select!(direction => {
                Direction::Taken => counter.saturating_add(1u8),
                Direction::Untaken => counter.saturating_sub(1u8),
            });
        }
        if direction == Direction::Taken {
            self.tags[index] = current;
            self.targets[index] = next;
            self.valid[index] = Value::new(1);
        }
    }

    fn size(&self) -> f64 {
        Array::<2, { Self::LENGTH }>::SIZE
            + 2.0 * Array::<64, { Self::LENGTH }>::SIZE
            + Array::<1, { Self::LENGTH }>::SIZE
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ramus::{Config, Costs, Info, Simulator, Source};
    use std::collections::VecDeque;

    struct Trace(VecDeque<Info<64>>);
    impl Source<64> for Trace {
        fn fetch(&mut self) -> Option<Info<64>> { self.0.pop_front() }
    }

    #[test]
    fn combined_direction_and_target_buffer() {
        let branch = |address, kind, direction, next| Info {
            address: Value::new(address), kind, direction, next: Value::new(next),
        };
        let trace = Trace(VecDeque::from([
            branch(0, Kind::UnconditionalDirect, Direction::Taken, 0), // Cold zero target: miss.
            branch(0, Kind::UnconditionalDirect, Direction::Taken, 0), // Hit.
            branch(0, Kind::Conditional, Direction::Untaken, 4), // Cold direction: hit.
            branch(0, Kind::UnconditionalDirect, Direction::Taken, 0), // Target preserved: hit.
            branch(4096, Kind::CallIndirect, Direction::Taken, 8), // Collision: miss.
            branch(0, Kind::Return, Direction::Taken, 0), // Evicted: miss.
            branch(0, Kind::Return, Direction::Taken, 16), // Changed target: miss.
            branch(0, Kind::Return, Direction::Taken, 16), // New target: hit.
            branch(0, Kind::Conditional, Direction::Taken, 16), // Counter 0: wrong direction.
            branch(0, Kind::Conditional, Direction::Taken, 16), // Counter 1: wrong direction.
            branch(0, Kind::Conditional, Direction::Taken, 16), // Counter 2: hit.
        ]));
        let metrics = Simulator::run_one(trace, SmithBimodal::default(), Costs::free(), Config::none());
        assert_eq!((metrics.predictions, metrics.hits), (11, 5));
        assert_eq!(metrics.space, 131.0 * 1024.0);
    }
}
