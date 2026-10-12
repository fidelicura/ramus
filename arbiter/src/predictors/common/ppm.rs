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
use ramus::{Array, Direction, Register, Value};

pub struct PPM {
    t0_counter: Array<3, { Self::T0 }>,
    t0_meta: Array<1, { Self::T0 }>,

    t1_counter: Array<3, { Self::T1 }>,
    t1_tag: Array<8, { Self::T1 }>,
    t1_usefulness: Array<1, { Self::T1 }>,

    t2_counter: Array<3, { Self::T2 }>,
    t2_tag: Array<8, { Self::T2 }>,
    t2_usefulness: Array<1, { Self::T2 }>,

    t3_counter: Array<3, { Self::T3 }>,
    t3_tag: Array<8, { Self::T3 }>,
    t3_usefulness: Array<1, { Self::T3 }>,

    t4_counter: Array<3, { Self::T4 }>,
    t4_tag: Array<8, { Self::T4 }>,
    t4_usefulness: Array<1, { Self::T4 }>,

    global_history: Register<64>,
}

impl Default for PPM {
    fn default() -> Self {
        Self {
            t0_counter: Default::default(),
            t0_meta: Default::default(),

            t1_counter: Default::default(),
            t1_tag: Default::default(),
            t1_usefulness: Default::default(),

            t2_counter: Default::default(),
            t2_tag: Default::default(),
            t2_usefulness: Default::default(),

            t3_counter: Default::default(),
            t3_tag: Default::default(),
            t3_usefulness: Default::default(),

            t4_counter: Default::default(),
            t4_tag: Default::default(),
            t4_usefulness: Default::default(),

            global_history: Default::default(),
        }
    }
}

impl PPM {
    const T0: usize = 4096;
    const T1: usize = 1024;
    const T2: usize = Self::T1;
    const T3: usize = Self::T1;
    const T4: usize = Self::T1;

    fn fold<const INPUT: usize, const OUTPUT: usize>(history: Value<INPUT>) -> Value<OUTPUT> {
        let mut folded = Value::<OUTPUT>::ZERO;
        for group in 0..INPUT.div_ceil(OUTPUT) {
            let chunk = (history >> (group as u8 * OUTPUT as u8)).resize::<OUTPUT>();
            folded = folded ^ chunk;
        }
        folded
    }

    fn index<const INPUT: usize, const OUTPUT: usize>(&self, index: Index<INPUT>) -> Value<OUTPUT> {
        // XXX: This `match` acts as a const-dispatch, so its execution costs are not
        // measured. This is done to avoid creating a function for each T* predictor.
        match index {
            Index::Local { address } => address.resize(),
            Index::Global { address, history } => {
                // XXX: This `match` is also not measured, for the same reason as above.
                let folded = match INPUT {
                    10 => history.resize::<10>(),
                    20 | 40 | 80 => Self::fold::<INPUT, 10>(history),
                    _ => panic!(),
                };

                (address ^ folded).resize()
            }
        }
    }

    fn tag<const INPUT: usize>(&self, address: Address, history: Value<INPUT>) -> Value<8> {
        let csr1 = Self::fold::<INPUT, 8>(history);
        let csr2 = Self::fold::<INPUT, 7>(history).resize::<8>();

        address.resize::<8>() ^ csr1 ^ (csr2 << 1u8)
    }

    pub fn predict(&self, address: Address) -> Direction {
        todo!()
    }
}

enum Index<const SIZE: usize> {
    Local {
        address: Value<12>,
    },
    Global {
        address: Value<10>,
        history: Value<SIZE>,
    },
}
