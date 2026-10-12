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
use ramus::{Array, Direction, Register, Value, overlap, select};
use std::collections::VecDeque;

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

    global_history: Register<80>,
    // Simulation bookkeeping: match each prediction-time lookup to its FIFO update.
    pending: VecDeque<(Address, Lookup)>,
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
            pending: Default::default(),
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

    fn lookup(&self, address: Address) -> Lookup {
        let (address_index, history_global) = overlap! {(
            address.resize::<10>() ^ (address >> 10u8).resize::<10>(),
            *self.global_history,
        )};

        let (t0_index, t1_index, t2_index, t3_index, t4_index) = overlap! {(
            self.index::<10, 12>(Index::Local {
                address: address.resize(),
            }),
            self.index::<10, 10>(Index::Global {
                address: address_index,
                history: history_global.resize::<10>(),
            }),
            self.index::<20, 10>(Index::Global {
                address: address_index,
                history: history_global.resize::<20>(),
            }),
            self.index::<40, 10>(Index::Global {
                address: address_index,
                history: history_global.resize::<40>(),
            }),
            self.index::<80, 10>(Index::Global {
                address: address_index,
                history: history_global,
            }),
        )};

        let (
            t0_counter,
            t1_counter,
            t2_counter,
            t3_counter,
            t4_counter,
            t1_tag_stored,
            t2_tag_stored,
            t3_tag_stored,
            t4_tag_stored,
            t1_tag_computed,
            t2_tag_computed,
            t3_tag_computed,
            t4_tag_computed,
        ) = overlap! {(
            self.t0_counter[t0_index],
            self.t1_counter[t1_index],
            self.t2_counter[t2_index],
            self.t3_counter[t3_index],
            self.t4_counter[t4_index],
            self.t1_tag[t1_index],
            self.t2_tag[t2_index],
            self.t3_tag[t3_index],
            self.t4_tag[t4_index],
            self.tag::<10>(address, history_global.resize::<10>()),
            self.tag::<20>(address, history_global.resize::<20>()),
            self.tag::<40>(address, history_global.resize::<40>()),
            self.tag::<80>(address, history_global),
        )};

        let (
            (t0_prediction, t1_prediction, t2_prediction, t3_prediction, t4_prediction),
            (t1_hit, t2_hit, t3_hit, t4_hit),
        ) = overlap! {(
            (
                t0_counter >= 4u8,
                t1_counter >= 4u8,
                t2_counter >= 4u8,
                t3_counter >= 4u8,
                t4_counter >= 4u8,
            ),
            (
                t1_tag_computed == t1_tag_stored,
                t2_tag_computed == t2_tag_stored,
                t3_tag_computed == t3_tag_stored,
                t4_tag_computed == t4_tag_stored,
            ),
        )};

        let (provider, prediction) = select!(t4_hit => {
            true => (4, t4_prediction),
            false => select!(t3_hit => {
                true => (3, t3_prediction),
                false => select!(t2_hit => {
                    true => (2, t2_prediction),
                    false => select!(t1_hit => {
                        true => (1, t1_prediction),
                        false => (0, t0_prediction),
                    }),
                }),
            }),
        });

        Lookup {
            base_index: t0_index,
            global_indices: [t1_index, t2_index, t3_index, t4_index],
            tags: [
                t1_tag_computed,
                t2_tag_computed,
                t3_tag_computed,
                t4_tag_computed,
            ],
            history: history_global,
            provider_number: provider,
            provider_prediction: prediction,
            base_prediction: t0_prediction,
        }
    }

    fn allocate(&mut self, address: Address, lookup: Lookup, direction: Direction, meta: Value<1>) {
        let (t1_usefulness, t2_usefulness, t3_usefulness, t4_usefulness) = select!(lookup.provider_number => {
            0 => overlap! {(
                self.t1_usefulness[lookup.global_indices[0]],
                self.t2_usefulness[lookup.global_indices[1]],
                self.t3_usefulness[lookup.global_indices[2]],
                self.t4_usefulness[lookup.global_indices[3]],
            )},
            1 => overlap! {(
                Value::new(1),
                self.t2_usefulness[lookup.global_indices[1]],
                self.t3_usefulness[lookup.global_indices[2]],
                self.t4_usefulness[lookup.global_indices[3]],
            )},
            2 => overlap! {(
                Value::new(1),
                Value::new(1),
                self.t3_usefulness[lookup.global_indices[2]],
                self.t4_usefulness[lookup.global_indices[3]],
            )},
            3 => overlap! {(
                Value::new(1),
                Value::new(1),
                Value::new(1),
                self.t4_usefulness[lookup.global_indices[3]],
            )},
            _ => unreachable!(),
        });
        let all_useful = t1_usefulness == 1u8
            && t2_usefulness == 1u8
            && t3_usefulness == 1u8
            && t4_usefulness == 1u8;

        // NOTE: Instead of writing RNG from scratch, hash PC and history for the victim choice.
        let victim_salt = address.resize::<16>() ^ lookup.history.resize::<16>();
        let victim_offset = victim_salt.wrapping_rem(4 - lookup.provider_number);

        let t1_victim = lookup.provider_number == 0 && victim_offset == 0u8;
        let t2_victim = (lookup.provider_number == 0 && victim_offset == 1u8)
            || (lookup.provider_number == 1 && victim_offset == 0u8);
        let t3_victim = (lookup.provider_number == 0 && victim_offset == 2u8)
            || (lookup.provider_number == 1 && victim_offset == 1u8)
            || (lookup.provider_number == 2 && victim_offset == 0u8);
        let t4_victim = (lookup.provider_number == 0 && victim_offset == 3u8)
            || (lookup.provider_number == 1 && victim_offset == 2u8)
            || (lookup.provider_number == 2 && victim_offset == 1u8)
            || lookup.provider_number == 3;

        let t1_allocate = lookup.provider_number == 0
            && if all_useful {
                t1_victim
            } else {
                t1_usefulness == 0u8
            };
        let t2_allocate = lookup.provider_number < 2
            && if all_useful {
                t2_victim
            } else {
                t2_usefulness == 0u8
            };
        let t3_allocate = lookup.provider_number < 3
            && if all_useful {
                t3_victim
            } else {
                t3_usefulness == 0u8
            };
        let t4_allocate = lookup.provider_number < 4
            && if all_useful {
                t4_victim
            } else {
                t4_usefulness == 0u8
            };

        let initial_taken = if meta.is_set() {
            bool::from(direction)
        } else {
            lookup.base_prediction
        };
        let initial_counter = if initial_taken {
            Value::new(4)
        } else {
            Value::new(3)
        };

        let (t1_index, t2_index, t3_index, t4_index) = overlap! {(
            lookup.global_indices[0],
            lookup.global_indices[1],
            lookup.global_indices[2],
            lookup.global_indices[3],
        )};

        overlap! {
            select!(t1_allocate => {
                true => overlap! {
                    self.t1_tag[t1_index] = lookup.tags[0];
                    self.t1_counter[t1_index] = initial_counter;
                    self.t1_usefulness[t1_index] = Value::ZERO;
                },
                false => {},
            });
            select!(t2_allocate => {
                true => overlap! {
                    self.t2_tag[t2_index] = lookup.tags[1];
                    self.t2_counter[t2_index] = initial_counter;
                    self.t2_usefulness[t2_index] = Value::ZERO;
                },
                false => {},
            });
            select!(t3_allocate => {
                true => overlap! {
                    self.t3_tag[t3_index] = lookup.tags[2];
                    self.t3_counter[t3_index] = initial_counter;
                    self.t3_usefulness[t3_index] = Value::ZERO;
                },
                false => {},
            });
            select!(t4_allocate => {
                true => overlap! {
                    self.t4_tag[t4_index] = lookup.tags[3];
                    self.t4_counter[t4_index] = initial_counter;
                    self.t4_usefulness[t4_index] = Value::ZERO;
                },
                false => {},
            });
        }
    }

    pub fn update(&mut self, address: Address, direction: Direction) {
        let (predicted_address, lookup) = self
            .pending
            .pop_front()
            .expect("PPM update should have a matching prediction");
        debug_assert_eq!(predicted_address, address);
        let actual = bool::from(direction);
        let correct = lookup.provider_prediction == actual;

        let counter = select!(lookup.provider_number => {
            0 => self.t0_counter[lookup.base_index],
            1 => self.t1_counter[lookup.global_indices[0]],
            2 => self.t2_counter[lookup.global_indices[1]],
            3 => self.t3_counter[lookup.global_indices[2]],
            4 => self.t4_counter[lookup.global_indices[3]],
            _ => unreachable!(),
        });
        let updated_counter = select!(direction => {
            Direction::Taken => counter.saturating_add(1u8),
            Direction::Untaken => counter.saturating_sub(1u8),
        });
        select!(lookup.provider_number => {
            0 => self.t0_counter[lookup.base_index] = updated_counter,
            1 => self.t1_counter[lookup.global_indices[0]] = updated_counter,
            2 => self.t2_counter[lookup.global_indices[1]] = updated_counter,
            3 => self.t3_counter[lookup.global_indices[2]] = updated_counter,
            4 => self.t4_counter[lookup.global_indices[3]] = updated_counter,
            _ => unreachable!(),
        });

        let needs_allocation = !correct && lookup.provider_number < 4;
        let meta = select!(needs_allocation => {
            true => self.t0_meta[lookup.base_index],
            false => Value::ZERO,
        });

        select!(lookup.provider_number > 0 && lookup.provider_prediction != lookup.base_prediction => {
            true => {
                let useful = Value::new(u128::from(correct));
                overlap! {
                    self.t0_meta[lookup.base_index] = useful;
                    select!(lookup.provider_number => {
                        1 => self.t1_usefulness[lookup.global_indices[0]] = useful,
                        2 => self.t2_usefulness[lookup.global_indices[1]] = useful,
                        3 => self.t3_usefulness[lookup.global_indices[2]] = useful,
                        4 => self.t4_usefulness[lookup.global_indices[3]] = useful,
                        _ => unreachable!(),
                    });
                }
            },
            false => {},
        });

        select!(needs_allocation => {
            true => self.allocate(address, lookup, direction, meta),
            false => {},
        });

        *self.global_history = (*self.global_history << 1u8) | Value::new(u128::from(actual));
    }

    pub fn predict(&mut self, address: Address) -> Direction {
        let lookup = self.lookup(address);
        self.pending.push_back((address, lookup));
        Direction::from(lookup.provider_prediction)
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

#[derive(Clone, Copy)]
struct Lookup {
    base_index: Value<12>,
    global_indices: [Value<10>; 4],
    tags: [Value<8>; 4],
    history: Value<80>,
    provider_number: u8,
    provider_prediction: bool,
    base_prediction: bool,
}
