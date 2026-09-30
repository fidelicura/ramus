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

use crate::predictors::{Address, Target};
use ramus::{Array, Direction, Kind, Prediction, Value, select};

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

    fn size(&self) -> f64 {
        Array::<2, { Self::LENGTH }>::SIZE
    }
}

#[derive(Default)]
struct BTB {
    buffer: Array<64, { Self::LENGTH }>,
} // Target: *b*ranch *t*arget *b*uffer.

impl BTB {
    /// Number of buckets in the table - one per branch address (after folding).
    ///
    /// Kept a power of two so the index can use a cheap AND mask (see `MASK`).
    const LENGTH: usize = 2usize.pow(12);

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

    fn predict(&self, address: Address) -> Target {
        self.buffer[Self::index(address)]
    }

    fn update(&mut self, address: Address, target: Target) {
        let index = Self::index(address);
        self.buffer[index] = target;
    }

    fn size(&self) -> f64 {
        Array::<64, { Self::LENGTH }>::SIZE
    }
}

#[derive(Default)]
struct RAS {
    stack: Array<64, { Self::LENGTH }>,
    index: Value<5>,
} // Stack: *r*eturn *a*ddress *s*tack.

impl RAS {
    /// Number of buckets in the table - one per branch address (after folding).
    ///
    /// Kept a power of two so the index can use a cheap AND mask (see `MASK`).
    const LENGTH: usize = 2usize.pow(5);

    fn push(&mut self, target: Target) {
        self.index = self.index.saturating_add(1u8);
        self.stack[self.index] = target;
    }

    fn pop(&mut self) {
        self.stack[self.index] = Target::ZERO;
        self.index = self.index.saturating_sub(1u8);
    }

    fn peek(&self) -> Target {
        self.stack[self.index]
    }

    fn update(&mut self, address: Address, kind: Kind) {
        select!(kind => {
            Kind::CallDirect | Kind::CallIndirect => self.push(address),
            Kind::Return => self.pop(),
            _ => unreachable!(),
        });
    }

    fn size(&self) -> f64 {
        Array::<64, { Self::LENGTH }>::SIZE + Value::<5>::SIZE
    }
}

#[derive(Default)]
struct Meta {
    table: Array<3, { BTB::LENGTH }>,
}

impl Meta {
    fn lookup(&self, address: Address) -> Kind {
        let index = BTB::index(address);
        Kind::from(self.table[index])
    }

    fn update(&mut self, address: Address, kind: Kind) {
        let index = BTB::index(address);
        self.table[index] = kind.into();
    }
}

#[derive(Default)]
pub struct Predictor {
    sct: SCT,
    btb: BTB,
    ras: RAS,
    meta: Meta,
}

impl ramus::Predictor<64> for Predictor {
    fn predict(&mut self, address: Address) -> Prediction<64> {
        let kind = self.meta.lookup(address);
        Prediction {
            direction: self.sct.predict(address),
            target: select!(kind => {
                Kind::Return => self.ras.peek(),
                _ => self.btb.predict(address),
            }),
        }
    }

    fn update(&mut self, current: Address, kind: Kind, direction: Direction, next: Address) {
        self.btb.update(current, next);
        select!(kind => {
            Kind::ConditionalDirect => self.sct.update(current, direction),
            Kind::UnconditionalDirect => self.meta.update(current, kind),
            Kind::UnconditionalIndirect => self.meta.update(current, kind),
            Kind::CallDirect => self.ras.update(current, kind),
            Kind::CallIndirect => self.ras.update(current, kind),
            Kind::Return => self.ras.update(current, kind),
        });
    }

    fn size(&self) -> f64 {
        self.sct.size() + self.btb.size() + self.ras.size()
    }
}
