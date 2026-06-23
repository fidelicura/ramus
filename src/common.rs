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

////////////////////////////////////////////////////////////////////////////////////////////////////
// Address {{
////////////////////////////////////////////////////////////////////////////////////////////////////

/// Instruction address of a branch.
pub type Address = Value<64>;

////////////////////////////////////////////////////////////////////////////////////////////////////
// }} Address
////////////////////////////////////////////////////////////////////////////////////////////////////

////////////////////////////////////////////////////////////////////////////////////////////////////
// Info {{
////////////////////////////////////////////////////////////////////////////////////////////////////

/// A single branch record drawn from a trace.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Info {
    /// Address of the branch instruction.
    pub address: Address,
    /// Whether the branch was taken.
    pub outcome: Outcome,
    /// Which kind of branch this is.
    pub kind: Kind,
    /// Next address; the target if taken, the fall-through address otherwise.
    pub next: Address,
}

////////////////////////////////////////////////////////////////////////////////////////////////////
// }} Info
////////////////////////////////////////////////////////////////////////////////////////////////////

////////////////////////////////////////////////////////////////////////////////////////////////////
// Kind {{
////////////////////////////////////////////////////////////////////////////////////////////////////

/// What kind of branch a record is.
///
/// Only [`Kind::Conditional`] is predicted and scored; the
/// rest are fed to [`crate::Predictor::track`] for history.
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
// Outcome {{
////////////////////////////////////////////////////////////////////////////////////////////////////

/// Whether a branch was taken.
#[derive(Debug, Clone, Copy, Hash, PartialEq, Eq, PartialOrd, Ord)]
pub enum Outcome {
    Taken,
    Untaken,
}

impl Default for Outcome {
    fn default() -> Self {
        Self::from(bool::default())
    }
}

impl From<bool> for Outcome {
    fn from(value: bool) -> Self {
        match value {
            true => Self::Taken,
            false => Self::Untaken,
        }
    }
}

impl From<Outcome> for bool {
    fn from(value: Outcome) -> Self {
        match value {
            Outcome::Taken => true,
            Outcome::Untaken => false,
        }
    }
}

////////////////////////////////////////////////////////////////////////////////////////////////////
// }} Outcome
////////////////////////////////////////////////////////////////////////////////////////////////////

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn outcome_conversions() {
        assert_eq!(Outcome::from(true), Outcome::Taken);
        assert_eq!(Outcome::from(false), Outcome::Untaken);

        assert!(bool::from(Outcome::Taken));
        assert!(!bool::from(Outcome::Untaken));
    }

    #[test]
    fn outcome_default() {
        assert_eq!(Outcome::from(bool::default()), Outcome::default());
        assert_eq!(Outcome::default(), Outcome::Untaken);
    }
}
