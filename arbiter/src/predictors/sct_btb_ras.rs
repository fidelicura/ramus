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

use crate::predictors::{
    Address,
    common::{BTB, RAS},
};
use ramus::{Array, Direction, Kind, Outcome, Prediction, Value, overlap, select};

#[derive(Default)]
struct SCT {
    table: Array<2, { Self::LENGTH }>,
} // Direction: *s*aturating *c*ounter *t*able.

impl SCT {
    /// Number of buckets in the table - one per branch address (after folding).
    ///
    /// Kept a power of two so the index can use a cheap AND mask (see `MASK`).
    const LENGTH: usize = 2usize.pow(10);

    /// A power-of-two table size lets us fold an address into a bucket with a
    /// single AND instead of a `%`: `addr & (LENGTH - 1)` is the bottom 10 bits.
    const MASK: u32 = Self::LENGTH as u32 - 1;

    /// Map a branch address to its bucket index.
    fn index(address: Address) -> Value<64> {
        // Instructions are 4-byte aligned, so the lowest 2 bits are always zero
        // and carry no information. Drop them first so neighbouring branches
        // don't all collide into the same bucket.
        let folded = address >> 2u8;
        // Keep only the lowest 10 bits => an index in 0..2^10 (0..1024).
        folded & Self::MASK
    }

    fn predict(&self, address: Address) -> Direction {
        let index = Self::index(address);
        let bucket = self.table[index];
        let outcome = bucket >> 1u8;
        Direction::from(outcome == 1u8)
    }

    fn update(&mut self, address: Address, direction: Direction) {
        let index = Self::index(address);
        let counter = self.table[index];
        self.table[index] = select!(direction => {
            Direction::Taken => counter.saturating_add(1u8),
            Direction::Untaken => counter.saturating_sub(1u8),
        });
    }

    const fn size(&self) -> f64 {
        Array::<2, { Self::LENGTH }>::SIZE
    }
}

#[derive(Default)]
struct Meta {
    table: Array<3, { Self::LENGTH }>,
    valid: Array<1, { Self::LENGTH }>,
}

impl Meta {
    const LENGTH: usize = 4096;

    fn lookup(&self, address: Address) -> Option<Kind> {
        let index = BTB::<{ Self::LENGTH }>::index(address);
        let valid = self.valid[index] == 1u8;
        valid.then(|| Kind::from(self.table[index]))
    }

    fn update(&mut self, address: Address, kind: Kind) {
        let index = BTB::<{ Self::LENGTH }>::index(address);
        overlap! {
            self.table[index] = Value::from(kind);
            self.valid[index] = Value::new(1);
        }
    }

    const fn size(&self) -> f64 {
        Array::<3, { Self::LENGTH }>::SIZE + Array::<1, { Self::LENGTH }>::SIZE
    }
}

#[derive(Default)]
pub struct Predictor {
    sct: SCT,
    btb: BTB<4096>,
    ras: RAS<32, 6>,
    meta: Meta,
}

impl ramus::Predictor<64> for Predictor {
    fn predict(&mut self, address: Address) -> Outcome<64> {
        let (kind, direction, guess_btb, guess_ras) = overlap! {(
            self.meta.lookup(address),
            self.sct.predict(address),
            self.btb.predict(address),
            self.ras.peek(),
        )};

        let target = select!(kind => {
            Some(kind) => select!(kind => {
                Kind::Return => select!(guess_ras => {
                    Some(guess_ras) => guess_ras,
                    None => guess_btb,
                }),
                _ => guess_btb,
            }),
            None => guess_btb,
        });

        Outcome::immediate()
            .set(Prediction { direction, target })
            .finish()
    }

    fn update(&mut self, current: Address, kind: Kind, direction: Direction, next: Address) {
        overlap! {
            select!(direction => {
                Direction::Taken => self.btb.update(current, next),
                Direction::Untaken => {},
            });
            self.sct.update(current, direction);
            select!(kind => {
                Kind::CallDirect => self.ras.update(next, kind),
                Kind::CallIndirect => self.ras.update(next, kind),
                Kind::Return => self.ras.update(current, kind),
                _ => {},
            });
            self.meta.update(current, kind);
        }
    }

    fn size(&self) -> f64 {
        self.sct.size() + self.btb.size() + self.ras.size() + self.meta.size()
    }
}
