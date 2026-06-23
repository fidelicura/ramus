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

use ramus::{Address, Outcome, Predictor, Value};

pub struct CoinRandom {
    state: Value<64>,
}

impl CoinRandom {
    // Any non-zero seed works; xorshift stalls forever at zero.
    const SEED: u128 = 0x2545_F491_4F6C_DD1D;

    // See <https://en.wikipedia.org/wiki/Xorshift>.
    fn xorshift(&mut self) -> Value<64> {
        let mut value = self.state;
        value = value ^ (value << 13u8);
        value = value ^ (value >> 7u8);
        value = value ^ (value << 17u8);
        self.state = value;
        value & 1u8
    }
}

impl Default for CoinRandom {
    fn default() -> Self {
        Self {
            state: Value::new(Self::SEED),
        }
    }
}

impl Predictor for CoinRandom {
    fn predict(&mut self, _: Address) -> Outcome {
        let prediction = self.xorshift();
        Outcome::from(prediction == 1u8)
    }

    fn size(&self) -> f64 {
        Value::<64>::SIZE
    }
}
