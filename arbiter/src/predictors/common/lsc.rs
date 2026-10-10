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
use ramus::{Array, Direction, Value, select};

pub struct LSC<const SIZE: usize, const CAPACITY: usize> {
    table: Array<SIZE, CAPACITY>,
}

impl<const SIZE: usize, const CAPACITY: usize> Default for LSC<SIZE, CAPACITY> {
    fn default() -> Self {
        const { Self::ASSERT };

        Self {
            table: Default::default(),
        }
    }
}

impl<const SIZE: usize, const CAPACITY: usize> LSC<SIZE, CAPACITY> {
    const ASSERT: () = const {
        assert!(SIZE != 0, "SIZE must not be zero");
        assert!(
            SIZE <= u8::MAX as usize,
            "SIZE must not be larger than `u8::MAX`"
        );

        assert!(CAPACITY != 0, "CAPACITY must not be zero");
        assert!(
            CAPACITY <= u64::MAX as usize,
            "CAPACITY must not be larger than `u64::MAX`"
        );
        assert!(
            CAPACITY.is_power_of_two(),
            "CAPACITY must be a power of two"
        );
    };

    /// [`index`] shifts away the two alignment bits, then ANDs with this mask
    /// to map the address into the table. Since `CAPACITY` is a power of two,
    /// `CAPACITY - 1` has its low `log2(CAPACITY)` bits set, selecting an index
    /// from `0..CAPACITY` without division, making this cheap and efficient.
    ///
    /// [`index`]: Self::index
    const MASK: u64 = (CAPACITY - 1) as u64;

    const SHIFT: u8 = (SIZE - 1) as u8;

    /// Map a branch `address` to its counter index.
    fn index(address: Address) -> Value<64> {
        // 1. Instructions are 4-byte aligned, so the lowest 2 bits are always
        // zero and carry no information. Drop them; now nearby instructions,
        // whose addresses differ by 4 bytes, differ in the low bits.
        let folded = address >> 2u8;

        // 2. Keep the low `log2(CAPACITY)` bits. For the number of control-flow
        // instructions that fit in this table, higher address bits are more
        // likely to be shared; retaining lower bits therefore distinguishes
        // more entries and reduces aliasing. The discarded high bits can still
        // cause addresses from different and far regions to map to the same counter.
        folded & Self::MASK
    }

    pub fn predict(&self, address: Address) -> Direction {
        let index = Self::index(address);

        let counter = self.table[index];

        let outcome = (counter >> Self::SHIFT) & 1u8;
        Direction::from(outcome == 1u8)
    }

    pub fn update(&mut self, address: Address, direction: Direction) {
        let index = Self::index(address);

        let counter = self.table[index];

        self.table[index] = select!(direction => {
            Direction::Taken => counter.saturating_add(1u8),
            Direction::Untaken => counter.saturating_sub(1u8),
        });
    }

    pub const fn size(&self) -> f64 {
        Array::<SIZE, CAPACITY>::SIZE
    }
}
