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

use ramus::{Address, Array, Outcome, Predictor, Value, select};

#[derive(Default)]
pub struct SmithBimodal {
    table: Array<2, { Self::LENGTH }>,
}

impl SmithBimodal {
    /// Number of buckets in the table - one per branch address (after folding).
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

impl Predictor for SmithBimodal {
    fn predict(&mut self, current: Address) -> Outcome {
        let index = Self::index(current);
        let counter = self.table[index];

        // Top bit of the 2-bit counter is the verdict: 2 or 3 => "taken".
        let taken = (counter >> 1u8) == 1u8;
        Outcome::from(taken)
    }

    fn update(&mut self, current: Address, outcome: Outcome, _next: Address) {
        let index = Self::index(current);
        let counter = self.table[index];

        // Saturating, so the counter sticks at the extremes (3 stays 3): that
        // hysteresis is what stops one stray outcome from flipping the verdict.
        // `select!` is `match` that also charges a 2-input multiplexer to the report.
        self.table[index] = select!(outcome => {
            Outcome::Taken   => counter.saturating_add(1u8),
            Outcome::Untaken => counter.saturating_sub(1u8),
        });
    }

    fn size(&self) -> f64 {
        Array::<2, { Self::LENGTH }>::SIZE
    }
}
