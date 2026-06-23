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

use ramus::{Address, Array, Outcome, Predictor, Register, Value, select};

#[derive(Default)]
pub struct GlobalSelect {
    register: Register<{ Self::HISTORY_BITS }>,
    table: Array<{ Self::TABLE_BITS }, { Self::TABLE_LENGTH }>,
}

impl GlobalSelect {
    const HISTORY_BITS: usize = 5;
    const ADDRESS_BITS: usize = 5;
    const TABLE_BITS: usize = 2;

    const INDEX_BITS: usize = Self::HISTORY_BITS + Self::ADDRESS_BITS;
    const ADDRESS_MASK: u32 = (1 << Self::ADDRESS_BITS) - 1;
    const TABLE_LENGTH: usize = 1 << Self::INDEX_BITS;

    fn index(history: Value<{ Self::HISTORY_BITS }>, current: Address) -> Address {
        // Instructions are 4-byte aligned, so the low 2 bits are always zero
        // and carry no information - drop them before taking the low address bits.
        let folded = (current >> 2u8) & Self::ADDRESS_MASK;

        (history.resize::<64>() << Self::ADDRESS_BITS as u8) | folded
    }
}

impl Predictor for GlobalSelect {
    fn predict(&mut self, current: Address) -> Outcome {
        let index = Self::index(*self.register, current);
        let counter = self.table[index];
        let prediction = (counter >> 1u8) == 1u8;
        Outcome::from(prediction)
    }

    fn update(&mut self, current: Address, outcome: Outcome, _: Address) {
        let history = *self.register;
        let index = Self::index(history, current);
        let counter = self.table[index];
        self.table[index] = select!(outcome => {
            Outcome::Taken => counter.saturating_add(1u8),
            Outcome::Untaken => counter.saturating_sub(1u8),
        });
        *self.register = select!(outcome => {
            Outcome::Taken => (history << 1u8) | 1u8,
            Outcome::Untaken => history << 1u8,
        });
    }

    fn size(&self) -> f64 {
        Register::<{ Self::HISTORY_BITS }>::SIZE
            + Array::<{ Self::TABLE_BITS }, { Self::TABLE_LENGTH }>::SIZE
    }
}
