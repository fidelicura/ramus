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
use ramus::{Array, Kind, Value, select};

pub struct RAS<const CAPACITY: usize, const COUNT: usize> {
    stack: Array<64, CAPACITY>,
    count: Value<COUNT>,
}

impl<const CAPACITY: usize, const COUNT: usize> Default for RAS<CAPACITY, COUNT> {
    fn default() -> Self {
        const { Self::ASSERT };

        Self {
            stack: Default::default(),
            count: Default::default(),
        }
    }
}

impl<const CAPACITY: usize, const COUNT: usize> RAS<CAPACITY, COUNT> {
    const ASSERT: () = const {
        assert!(CAPACITY != 0, "CAPACITY must not be zero");
        assert!(
            CAPACITY <= u8::MAX as usize,
            "CAPACITY must not be larger than `u8::MAX`"
        );

        assert!(COUNT != 0, "COUNT must not be zero");
        assert!(
            COUNT <= u8::MAX as usize,
            "COUNT must not be larger than `u8::MAX`"
        );

        assert!(
            COUNT == CAPACITY.ilog2() as usize + 1,
            "COUNT must equal floor(log2(CAPACITY)) + 1"
        );
    };

    pub fn push(&mut self, target: Target) {
        const { Self::ASSERT };

        select!(self.count < CAPACITY as u8 => {
            true => {
                self.stack[self.count] = target;
                self.count = self.count.saturating_add(1u8);
            },
            false => {},
        });
    }

    pub fn pop(&mut self) {
        select!(self.count > 0u8 => {
            true => {
                self.count = self.count.saturating_sub(1u8);
                self.stack[self.count] = Target::ZERO;
            },
            false => {},
        });
    }

    pub fn peek(&self) -> Option<Target> {
        select!(self.count > 0u8 => {
            true => Some(self.stack[self.count.saturating_sub(1u8)]),
            false => None,
        })
    }

    pub fn update(&mut self, address: Address, kind: Kind) {
        select!(kind => {
            Kind::CallDirect | Kind::CallIndirect => self.push(address),
            Kind::Return => self.pop(),
            _ => unreachable!(),
        });
    }

    pub const fn size(&self) -> f64 {
        Array::<64, CAPACITY>::SIZE + Value::<COUNT>::SIZE
    }
}
