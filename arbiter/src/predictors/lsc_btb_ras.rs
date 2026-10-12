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
    common::{BTB, LSC, RAS},
};
use ramus::{Array, Direction, Kind, Outcome, Prediction, Value, overlap, select};

#[derive(Default)]
struct Meta {
    table: Array<3, { Self::LENGTH }>,
    valid: Array<1, { Self::LENGTH }>,
}

impl Meta {
    const LENGTH: usize = 4096;

    fn predict(&self, address: Address) -> Option<Kind> {
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
    lsc: LSC<2, 1024>,
    btb: BTB<4096>,
    ras: RAS<32, 6>,
    meta: Meta,
}

impl ramus::Predictor<64> for Predictor {
    fn predict(&mut self, address: Address) -> Outcome<64> {
        let (kind, direction, guess_btb, guess_ras) = overlap! {(
            self.meta.predict(address),
            self.lsc.predict(address),
            self.btb.predict(address),
            self.ras.predict(),
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
            self.lsc.update(current, direction);
            select!(kind => {
                Kind::CallDirect | Kind::CallIndirect => self.ras.update(next, kind),
                Kind::Return => self.ras.update(current, kind),
                _ => {},
            });
            self.meta.update(current, kind);
        }
    }

    fn size(&self) -> f64 {
        self.lsc.size() + self.btb.size() + self.ras.size() + self.meta.size()
    }
}
