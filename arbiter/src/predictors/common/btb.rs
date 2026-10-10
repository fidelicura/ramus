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
use ramus::{Array, Value, overlap};

/// # Description
///
/// Direct-mapped branch target buffer with a sequential-address fallback.
///
/// # Implementation
///
/// The table index drops the address's two instruction-alignment bits, then
/// masks the result to the capacity. Each index stores one target and a valid
/// bit; an invalid entry predicts the next sequential address (`address + 4`).
///
/// # Considerations
///
/// - [`CAPACITY`] must be a non-zero power of two.
/// - Entries have no address tags, so addresses that fold to the same index alias.
pub struct BTB<const CAPACITY: usize> {
    buffer: Array<64, CAPACITY>,
    valid: Array<1, CAPACITY>,
}

impl<const CAPACITY: usize> Default for BTB<CAPACITY> {
    fn default() -> Self {
        const { Self::ASSERT };

        Self {
            buffer: Default::default(),
            valid: Default::default(),
        }
    }
}

impl<const CAPACITY: usize> BTB<CAPACITY> {
    const ASSERT: () = const {
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

    /// Map a branch address to its bucket index.
    pub fn index(address: Address) -> Value<64> {
        // 1. Instructions are 4-byte aligned, so the lowest 2 bits are always
        // zero and carry no information. Drop them; now nearby instructions,
        // whose addresses differ by 4 bytes, differ in the low bits.
        let folded = address >> 2u8;

        // 2. Keep the low `log2(CAPACITY)` bits. For the number of control-flow
        // instructions that fit in this table, higher address bits are more
        // likely to be shared; retaining lower bits therefore distinguishes
        // more entries and reduces aliasing. The discarded high bits can still
        // cause addresses from different and far regions to map to the same bucket.
        folded & Self::MASK
    }

    /// Predict the target associated with `address`.
    pub fn predict(&self, address: Address) -> Target {
        // 1. Map the branch address to its direct-mapped bucket.
        let index = Self::index(address);

        // 2. Check validity separately from the target value: address zero
        // is a valid target, so it cannot double as an "unused" sentinel.
        let validity = self.valid[index];

        // 3. On a hit, use the stored target. On a miss, fall through to
        // address + 4, the common path that needs no target-table entry.
        // Without tags, a hit can be an alias from another branch address.
        if validity.is_set() {
            self.buffer[index]
        } else {
            address.wrapping_add(4u8)
        }
    }

    /// Update the bucket selected by `address` with `target`.
    pub fn update(&mut self, address: Address, target: Target) {
        // 1. Use the same mapping as `predict`, so this write is visible to
        // the next lookup of this address (and of any colliding address).
        let index = Self::index(address);

        // 2. Write the data and mark it valid together. Separate arrays
        // mean these independent writes can happen in parallel.
        overlap! {
            self.buffer[index] = target;
            self.valid[index] = Value::new(1);
        };
    }

    pub const fn size(&self) -> f64 {
        Array::<64, CAPACITY>::SIZE + Array::<1, CAPACITY>::SIZE
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ramus::{
        Config, Costs, Direction, Info, Kind, Outcome, Prediction, Predictor, Simulator, Source,
    };
    use std::{cell::RefCell, rc::Rc};

    struct Trace(Option<Info<64>>);

    impl Source<64> for Trace {
        fn fetch(&mut self) -> Option<Info<64>> {
            self.0.take()
        }
    }

    struct Probe<const CAPACITY: usize> {
        btb: Rc<RefCell<BTB<CAPACITY>>>,
        predictions: Rc<RefCell<Vec<Target>>>,
    }

    impl<const CAPACITY: usize> Predictor<64> for Probe<CAPACITY> {
        fn predict(&mut self, address: Address) -> Outcome<64> {
            let target = self.btb.borrow().predict(address);
            self.predictions.borrow_mut().push(target);
            Outcome::immediate()
                .set(Prediction {
                    direction: Direction::Taken,
                    target,
                })
                .finish()
        }

        fn update(&mut self, address: Address, _: Kind, direction: Direction, next: Address) {
            if matches!(direction, Direction::Taken) {
                self.btb.borrow_mut().update(address, next);
            }
        }

        fn size(&self) -> f64 {
            self.btb.borrow().size()
        }
    }

    fn run<const CAPACITY: usize>(btb: Rc<RefCell<BTB<CAPACITY>>>, info: Info<64>) -> Target {
        let predictions = Rc::new(RefCell::new(Vec::new()));
        Simulator::run_one(
            Trace(Some(info)),
            Probe {
                btb,
                predictions: predictions.clone(),
            },
            Costs::free(),
            Config::none(),
        );
        *predictions.borrow().last().unwrap()
    }

    fn branch(address: u64, next: u64) -> Info<64> {
        Info {
            address: Address::new(address as u128),
            direction: Direction::Taken,
            kind: Kind::ConditionalDirect,
            next: Target::new(next as u128),
        }
    }

    #[test]
    fn miss_falls_through_then_taken_update_is_used() {
        let btb = Rc::new(RefCell::new(BTB::<4>::default()));
        let address = Address::new(0x100);
        let target = Target::new(0x800);

        assert_eq!(
            run(btb.clone(), branch(0x100, 0x800)),
            address.wrapping_add(4u8)
        );
        assert_eq!(run(btb, branch(0x100, 0x800)), target);
    }

    #[test]
    fn colliding_addresses_share_and_replace_an_entry() {
        let btb = Rc::new(RefCell::new(BTB::<4>::default()));
        let first_target = Target::new(0x800);
        let collision_target = Target::new(0x900);

        let _ = run(btb.clone(), branch(0x100, 0x800));
        assert_eq!(run(btb.clone(), branch(0x110, 0x900)), first_target);
        let _ = run(btb.clone(), branch(0x110, 0x900));
        assert_eq!(run(btb, branch(0x100, 0x800)), collision_target);
    }
}
