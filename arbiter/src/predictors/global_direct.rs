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

use ramus::{Address, Array, Outcome, Predictor, Register, select};

#[derive(Default)]
pub struct GlobalDirect {
    register: Register<{ Self::REGISTER_BITS }>,
    table: Array<{ Self::TABLE_BITS }, { Self::TABLE_LENGTH }>,
}

impl GlobalDirect {
    const REGISTER_BITS: usize = 10;
    const TABLE_BITS: usize = 2;
    const TABLE_LENGTH: usize = 1 << Self::REGISTER_BITS;
}

impl Predictor for GlobalDirect {
    fn predict(&mut self, _: Address) -> Outcome {
        let index = *self.register;
        let counter = self.table[index];
        let prediction = (counter >> 1u8) == 1u8;
        Outcome::from(prediction)
    }

    fn update(&mut self, _: Address, outcome: Outcome, _: Address) {
        let index = *self.register;
        let counter = self.table[index];
        self.table[index] = select!(outcome => {
            Outcome::Taken => counter.saturating_add(1u8),
            Outcome::Untaken => counter.saturating_sub(1u8),
        });
        *self.register = select!(outcome => {
            Outcome::Taken => (index << 1u8) | 1u8,
            Outcome::Untaken => index << 1u8,
        });
    }

    fn size(&self) -> f64 {
        Register::<{ Self::REGISTER_BITS }>::SIZE
            + Array::<{ Self::TABLE_BITS }, { Self::TABLE_LENGTH }>::SIZE
    }
}
