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

use crate::observability::{Costs, Metrics, Operation};
use crate::primitives::value::Value;
use std::{array, ops};

////////////////////////////////////////////////////////////////////////////////////////////////////
// Array {{
////////////////////////////////////////////////////////////////////////////////////////////////////

/// Fixed-size array of `LENGTH` cells, each a [`Value`] of `BITS` width.
///
/// Indexed by a [`Value`] rather than `usize` so reads and writes
/// flow through the same bit-width type as the data they address.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Array<const BITS: usize, const LENGTH: usize>([Value<BITS>; LENGTH]);

impl<const BITS: usize, const LENGTH: usize> Array<BITS, LENGTH> {
    /// Total static area of the array: per-cell width
    /// times cell count, in the cost model's area units.
    pub const SIZE: f64 = Value::<BITS>::SIZE * LENGTH as f64;

    /// Creates an array with every cell set to [`Value::ZERO`].
    pub fn new() -> Self {
        Self(array::from_fn(|_| Value::ZERO))
    }
}

impl<const BITS: usize, const LENGTH: usize> Default for Array<BITS, LENGTH> {
    fn default() -> Self {
        Self::new()
    }
}

impl<const ARRAY_BITS: usize, const ARRAY_LENGTH: usize, const VALUE_BITS: usize>
    ops::Index<Value<VALUE_BITS>> for Array<ARRAY_BITS, ARRAY_LENGTH>
{
    type Output = Value<ARRAY_BITS>;

    fn index(&self, index: Value<VALUE_BITS>) -> &Self::Output {
        Metrics::charge::<ARRAY_BITS>(Operation::memory_read, Costs::get().memory_read);
        let index = usize::try_from(index.0).expect("u128 value does not fit usize range");
        &self.0[index]
    }
}

impl<const ARRAY_BITS: usize, const ARRAY_LENGTH: usize, const VALUE_BITS: usize>
    ops::IndexMut<Value<VALUE_BITS>> for Array<ARRAY_BITS, ARRAY_LENGTH>
{
    fn index_mut(&mut self, index: Value<VALUE_BITS>) -> &mut Self::Output {
        Metrics::charge::<ARRAY_BITS>(Operation::memory_write, Costs::get().memory_write);
        let index = usize::try_from(index.0).expect("u128 value does not fit usize range");
        &mut self.0[index]
    }
}

////////////////////////////////////////////////////////////////////////////////////////////////////
// }} Array
////////////////////////////////////////////////////////////////////////////////////////////////////

#[cfg(test)]
mod tests {
    use super::*;
    use crate::observability::{Cost, Metrics};

    #[test]
    fn array_indexed_by_value() {
        Costs::set(Costs::free());
        let mut array = Array::<8, 4>::default();
        assert_eq!(array, Array::<8, 4>::new());

        let index = Value::<8>::wrapping(2u16);
        assert_eq!(array[index], Value::ZERO);
        array[index] = Value::wrapping(7u16);
        assert_eq!(array[index], 7u32);
    }

    #[test]
    fn index_charges_read_and_index_mut_charges_write() {
        // Price read and write apart so the totals attribute each port.
        Costs::set(Costs {
            memory_read: Cost {
                switching_energy: Some(2.0),
                ..Cost::free()
            },
            memory_write: Cost {
                switching_energy: Some(5.0),
                ..Cost::free()
            },
            ..Costs::free()
        });
        Metrics::reset();

        let mut array = Array::<8, 4>::default();
        let index = Value::<8>::wrapping(1u16);
        let _ = array[index]; // A read through plain `Index`.
        array[index] = Value::wrapping(3u16); // A write through `IndexMut`.
        let _ = array[index]; // A read through plain `Index`.

        assert_eq!(Metrics::reset().energy, 9.0); // 2 reads * 2 + 1 write * 5.
        Costs::set(Costs::default());
    }
}
