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
pub struct LastOutcome {
    buckets: Array<1, { Self::LENGTH }>,
}

impl LastOutcome {
    const LENGTH: usize = 1024;

    // Array size is a power of two, thus subtraction of one from
    // it creates a low-bit mask, which is then used as the cheap
    // power-of-two modulo that folds the address into the table.
    const MASK: u32 = Self::LENGTH as u32 - 1;

    fn index(address: Address) -> Value<64> {
        // Instructions are four byte aligned, so last two bits are always
        // zero and carry no useful information. Shifting them out keeps
        // adjacent instructions from colliding into the same slot.
        let unique = address >> 2u8;

        unique & Self::MASK
    }
}

impl Predictor for LastOutcome {
    fn predict(&mut self, current: Address) -> Outcome {
        let index = Self::index(current);
        let bucket = self.buckets[index];
        let prediction = bucket == 1u8;
        Outcome::from(prediction)
    }

    fn update(&mut self, current: Address, outcome: Outcome, _: Address) {
        let index = Self::index(current);
        self.buckets[index] = select!(outcome => {
            Outcome::Taken => Value::new(1),
            Outcome::Untaken => Value::new(0),
        });
    }

    fn size(&self) -> f64 {
        Array::<1, { Self::LENGTH }>::SIZE
    }
}
