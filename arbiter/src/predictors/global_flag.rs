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

use ramus::{Address, Outcome, Predictor, Register, select};

#[derive(Default)]
pub struct GlobalFlag {
    flag: Register<1>,
}

impl Predictor for GlobalFlag {
    fn predict(&mut self, _: Address) -> Outcome {
        let prediction = *self.flag == 1u8;
        Outcome::from(prediction)
    }

    fn update(&mut self, _: Address, outcome: Outcome, _: Address) {
        *self.flag = select!(outcome => {
            Outcome::Taken => self.flag.saturating_add(1u8),
            Outcome::Untaken => self.flag.saturating_sub(1u8)
        });
    }

    fn size(&self) -> f64 {
        Register::<1>::SIZE
    }
}
