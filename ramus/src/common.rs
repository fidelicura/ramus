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

use crate::observability::Metrics;
use crate::primitives::Value;
use std::ops;

////////////////////////////////////////////////////////////////////////////////////////////////////
// Address {{
////////////////////////////////////////////////////////////////////////////////////////////////////

/// Instruction address of a branch, `BITS` wide.
pub type Address<const BITS: usize> = Value<BITS>;

////////////////////////////////////////////////////////////////////////////////////////////////////
// }} Address
////////////////////////////////////////////////////////////////////////////////////////////////////

////////////////////////////////////////////////////////////////////////////////////////////////////
// Info {{
////////////////////////////////////////////////////////////////////////////////////////////////////

/// A single branch record drawn from a trace of `BITS`-wide addresses.
#[derive(Debug, Clone, Copy)]
pub struct Info<const BITS: usize> {
    /// Address of the branch instruction.
    pub address: Address<BITS>,
    /// Whether the branch was taken.
    pub direction: Direction,
    /// Which kind of branch this is.
    pub kind: Kind,
    /// Next address; the target if taken, the fall-through address otherwise.
    pub next: Address<BITS>,
}

////////////////////////////////////////////////////////////////////////////////////////////////////
// }} Info
////////////////////////////////////////////////////////////////////////////////////////////////////

////////////////////////////////////////////////////////////////////////////////////////////////////
// Outcome {{
////////////////////////////////////////////////////////////////////////////////////////////////////

#[derive(Debug, Default, Clone)]
pub struct Outcome<const BITS: usize>(outcome::Storage<BITS>);

impl<const BITS: usize> Outcome<BITS> {
    pub(crate) const fn new(storage: outcome::Storage<BITS>) -> Self {
        Self(storage)
    }

    pub(crate) fn collate(&self, info: Info<BITS>) -> (bool, bool, bool) {
        let goal = Prediction {
            direction: info.direction,
            target: info.next,
        };
        self.0.collate(goal)
    }

    pub fn immediate() -> outcome::Immediate<BITS> {
        outcome::Immediate::default()
    }

    pub fn staged() -> outcome::Staged<BITS> {
        outcome::Staged::default()
    }
}

mod outcome {
    use super::*;

    #[derive(Debug, Default, Clone)]
    pub struct Storage<const BITS: usize> {
        payload: Vec<(u64, Prediction<BITS>)>,
    }

    impl<const BITS: usize> Storage<BITS> {
        pub const fn new() -> Self {
            Self {
                payload: Vec::new(),
            }
        }

        pub fn put(&mut self, tuple: (u64, Prediction<BITS>)) {
            self.payload.push(tuple);
        }

        pub fn len(&self) -> usize {
            self.payload.len()
        }

        /// Score the staged predictions against the actual outcome.
        ///
        /// Score the greatest-delay prediction's direction and next address.
        /// Timing uses that same prediction's delay.
        pub fn collate(&self, goal: Prediction<BITS>) -> (bool, bool, bool) {
            let (prediction_delay, final_prediction) = self
                .payload
                .iter()
                .max_by_key(|(delay, _)| *delay)
                .expect("outcome must contain at least one prediction");

            let target_hit = final_prediction.target.0 == goal.target.0;
            let direction_hit = final_prediction.direction.0 == goal.direction.0;
            let exact_hit = direction_hit && target_hit;

            Metrics::push(*prediction_delay);
            (exact_hit, direction_hit, target_hit)
        }
    }

    #[derive(Debug, Default, Clone)]
    pub struct Immediate<const BITS: usize> {
        pub payload: Option<(u64, Prediction<BITS>)>,
    }

    impl<const BITS: usize> Immediate<BITS> {
        pub fn set(self, prediction: Prediction<BITS>) -> Self {
            let payload = (0u64, prediction);
            Self {
                payload: Some(payload),
            }
        }

        pub fn finish(self) -> Outcome<BITS> {
            let mut storage = Storage::new();
            let payload = self.payload.expect("TODO");
            storage.put(payload);
            Outcome::new(storage)
        }
    }

    #[derive(Debug, Default, Clone)]
    pub struct Staged<const BITS: usize>(Storage<BITS>);

    impl<const BITS: usize> Staged<BITS> {
        pub fn at(&mut self, delay: u64, prediction: Prediction<BITS>) {
            let payload = (delay, prediction);
            self.0.put(payload);
        }

        pub fn finish(self) -> Outcome<BITS> {
            assert_ne!(self.0.len(), 0, "TODO");
            Outcome(self.0)
        }
    }
}

////////////////////////////////////////////////////////////////////////////////////////////////////
// }} Outcome
////////////////////////////////////////////////////////////////////////////////////////////////////

////////////////////////////////////////////////////////////////////////////////////////////////////
// Prediction {{
////////////////////////////////////////////////////////////////////////////////////////////////////

/// Predicted direction and taken target for one branch.
#[derive(Debug, Clone, Copy)]
pub struct Prediction<const BITS: usize> {
    pub direction: Direction,
    pub target: Target<BITS>,
}

////////////////////////////////////////////////////////////////////////////////////////////////////
// }} Prediction
////////////////////////////////////////////////////////////////////////////////////////////////////

////////////////////////////////////////////////////////////////////////////////////////////////////
// Kind {{
////////////////////////////////////////////////////////////////////////////////////////////////////

/// What kind of branch a record is.
///
/// Every kind is predicted, scored, and resolved through the same predictor.
//
// XXX: The starting discriminant is intentionally set to 0.
// This avoids clashing with `Value::ZERO` as an uninitialized state.
#[repr(u8)]
#[derive(Debug, Clone, Copy)]
pub enum Kind {
    /// Conditional direct branch (taken or not depending on a condition).
    ConditionalDirect = 1,
    /// Unconditional direct branch (target encoded in the instruction).
    UnconditionalDirect = 2,
    /// Unconditional indirect branch (target from a register).
    UnconditionalIndirect = 3,
    /// Direct call of routine.
    CallDirect = 4,
    /// Indirect call of routine.
    CallIndirect = 5,
    /// Return from routine.
    Return = 6,
}

impl Kind {
    pub const fn is_conditional(&self) -> bool {
        matches!(self, Self::ConditionalDirect)
    }

    pub const fn is_indirect(&self) -> bool {
        matches!(self, Self::UnconditionalIndirect | Self::CallIndirect)
    }
}

impl<const BITS: usize> From<Value<BITS>> for Kind {
    fn from(value: Value<BITS>) -> Self {
        match value.0 {
            1 => Self::ConditionalDirect,
            2 => Self::UnconditionalDirect,
            3 => Self::UnconditionalIndirect,
            4 => Self::CallDirect,
            5 => Self::CallIndirect,
            6 => Self::Return,
            discriminant => panic!("wrong discriminant for `Kind`: {discriminant}"),
        }
    }
}

impl<const BITS: usize> From<Kind> for Value<BITS> {
    fn from(value: Kind) -> Self {
        match value {
            Kind::ConditionalDirect => Self::new(1),
            Kind::UnconditionalDirect => Self::new(2),
            Kind::UnconditionalIndirect => Self::new(3),
            Kind::CallDirect => Self::new(4),
            Kind::CallIndirect => Self::new(5),
            Kind::Return => Self::new(6),
        }
    }
}

////////////////////////////////////////////////////////////////////////////////////////////////////
// }} Kind
////////////////////////////////////////////////////////////////////////////////////////////////////

////////////////////////////////////////////////////////////////////////////////////////////////////
// Direction {{
////////////////////////////////////////////////////////////////////////////////////////////////////

/// Whether a branch was taken.
// #[derive(Debug, Clone, Copy, Hash, PartialEq, Eq, PartialOrd, Ord)]
#[derive(Debug, Clone, Copy)]
pub enum Direction {
    Taken,
    Untaken,
}

impl Default for Direction {
    fn default() -> Self {
        let value = bool::default();
        Self::from(value)
    }
}

impl From<bool> for Direction {
    fn from(value: bool) -> Self {
        match value {
            true => Self::Taken,
            false => Self::Untaken,
        }
    }
}

impl From<Direction> for bool {
    fn from(value: Direction) -> Self {
        match value {
            Direction::Taken => true,
            Direction::Untaken => false,
        }
    }
}

impl ops::Not for Direction {
    type Output = Self;

    fn not(self) -> Self::Output {
        match self {
            Self::Taken => Self::Untaken,
            Self::Untaken => Self::Taken,
        }
    }
}

impl ops::Deref for Direction {
    type Target = Value<1>;

    fn deref(&self) -> &Self::Target {
        match self {
            Self::Taken => &Value(1),
            Self::Untaken => &Value(0),
        }
    }
}

////////////////////////////////////////////////////////////////////////////////////////////////////
// }} Direction
////////////////////////////////////////////////////////////////////////////////////////////////////

////////////////////////////////////////////////////////////////////////////////////////////////////
// Target {{
////////////////////////////////////////////////////////////////////////////////////////////////////

pub type Target<const BITS: usize> = Address<BITS>;

////////////////////////////////////////////////////////////////////////////////////////////////////
// }} Target
////////////////////////////////////////////////////////////////////////////////////////////////////

////////////////////////////////////////////////////////////////////////////////////////////////////
// validate_bits! {{
////////////////////////////////////////////////////////////////////////////////////////////////////

/// Compile-time guard: associating with this constant fails
/// the build if `BITS` exceeds the 128-bit backing store.
#[macro_export]
macro_rules! validate_bits {
    (self) => {
        const { Self::VALID };
    };

    ($bits:ident) => {
        #[allow(dead_code)] // XXX: It is not "dead" - it still asserts in const contexts.
        const VALID: () = assert!(
            $bits > 0 && $bits <= 128,
            "BITS should be in range 0 < BITS <= 128"
        );
    };
}

////////////////////////////////////////////////////////////////////////////////////////////////////
// }} validate_bits!
////////////////////////////////////////////////////////////////////////////////////////////////////

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn direction_conversions() {
        assert_eq!(Direction::from(true).0, Direction::Taken.0);
        assert_eq!(Direction::from(false).0, Direction::Untaken.0);

        assert!(bool::from(Direction::Taken));
        assert!(!bool::from(Direction::Untaken));
    }

    #[test]
    fn direction_default() {
        assert_eq!(Direction::from(bool::default()).0, Direction::default().0);
        assert_eq!(Direction::default().0, Direction::Untaken.0);
    }

    #[test]
    fn direction_inversions() {
        assert_eq!(Direction::Untaken.0, (!Direction::Taken).0);
        assert_eq!(Direction::Taken.0, (!Direction::Untaken).0);
    }
}
