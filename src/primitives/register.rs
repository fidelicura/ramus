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
use std::ops;

////////////////////////////////////////////////////////////////////////////////////////////////////
// Register {{
////////////////////////////////////////////////////////////////////////////////////////////////////

/// A single [`Value`] of `BITS` width, metered like [`Array`](crate::Array) but
/// without an index - one cell of predictor state instead of a table of them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Register<const BITS: usize>(Value<BITS>);

impl<const BITS: usize> Register<BITS> {
    /// Static area of one register: its `BITS` width, in the cost model's area units.
    pub const SIZE: f64 = Value::<BITS>::SIZE;

    /// Creates a register set to [`Value::ZERO`].
    pub fn new() -> Self {
        Self(Value::ZERO)
    }
}

impl<const BITS: usize> Default for Register<BITS> {
    fn default() -> Self {
        Self::new()
    }
}

impl<const BITS: usize> ops::Deref for Register<BITS> {
    type Target = Value<BITS>;

    fn deref(&self) -> &Self::Target {
        Metrics::charge::<BITS>(Operation::register_read, Costs::get().register_read);
        &self.0
    }
}

impl<const BITS: usize> ops::DerefMut for Register<BITS> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        Metrics::charge::<BITS>(Operation::register_write, Costs::get().register_write);
        &mut self.0
    }
}

////////////////////////////////////////////////////////////////////////////////////////////////////
// }} Register
////////////////////////////////////////////////////////////////////////////////////////////////////

#[cfg(test)]
mod tests {
    use super::*;
    use crate::observability::{Cost, Metrics};

    #[test]
    fn register_derefs_to_its_value() {
        Costs::set(Costs::free());
        let mut register = Register::<8>::default();
        assert_eq!(*register, Value::ZERO);

        *register = Value::wrapping(7u16);
        assert_eq!(*register, 7u32);
    }

    #[test]
    fn deref_charges_read_and_deref_mut_charges_write() {
        Costs::set(Costs {
            register_read: Cost {
                switching_energy: Some(2.0),
                ..Cost::free()
            },
            register_write: Cost {
                switching_energy: Some(5.0),
                ..Cost::free()
            },
            ..Costs::free()
        });
        Metrics::reset();

        let mut register = Register::<8>::default();
        let _ = *register; // A read through `Deref`.
        *register = Value::wrapping(3u16); // A write through `DerefMut`.
        let _ = *register; // A read through `Deref`.

        assert_eq!(Metrics::reset().energy, 9.0); // 2 reads * 2 + 1 write * 5.
        Costs::set(Costs::default());
    }
}
