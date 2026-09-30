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
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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
// Prediction {{
////////////////////////////////////////////////////////////////////////////////////////////////////

/// Predicted direction and taken target for one branch.
#[derive(Debug, Clone, Copy)]
pub struct Prediction<const BITS: usize> {
    pub direction: Direction,
    /// `None` means the target is unknown (for example, a cold buffer entry).
    /// Ignored when the branch is correctly predicted untaken.
    pub target: Option<Address<BITS>>,
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
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// Conditional branch (taken or not depending on a condition).
    Conditional,
    /// Unconditional direct branch (target encoded in the instruction).
    UnconditionalDirect,
    /// Unconditional indirect branch (target from a register).
    UnconditionalIndirect,
    /// Direct call of routine.
    CallDirect,
    /// Indirect call of routine.
    CallIndirect,
    /// Return from routine.
    Return,
}

////////////////////////////////////////////////////////////////////////////////////////////////////
// }} Kind
////////////////////////////////////////////////////////////////////////////////////////////////////

////////////////////////////////////////////////////////////////////////////////////////////////////
// Direction {{
////////////////////////////////////////////////////////////////////////////////////////////////////

/// Whether a branch was taken.
#[derive(Debug, Clone, Copy, Hash, PartialEq, Eq, PartialOrd, Ord)]
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn direction_conversions() {
        assert_eq!(Direction::from(true), Direction::Taken);
        assert_eq!(Direction::from(false), Direction::Untaken);

        assert!(bool::from(Direction::Taken));
        assert!(!bool::from(Direction::Untaken));
    }

    #[test]
    fn direction_default() {
        assert_eq!(Direction::from(bool::default()), Direction::default());
        assert_eq!(Direction::default(), Direction::Untaken);
    }

    #[test]
    fn direction_inversions() {
        assert_eq!(Direction::Untaken, !Direction::Taken);
        assert_eq!(Direction::Taken, !Direction::Untaken);
    }
}
