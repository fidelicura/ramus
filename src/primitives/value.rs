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
use std::cmp::{self, Ordering};
use std::ops;

////////////////////////////////////////////////////////////////////////////////////////////////////
// Value {{
////////////////////////////////////////////////////////////////////////////////////////////////////

/// An unsigned integer constrained to `BITS` width, stored in a `u128`.
///
/// The invariant is that the backing value never has any bit set
/// above `BITS`. Construct via `Value::{new,wrapping,failing}`.
#[derive(Debug, Clone, Copy)]
#[must_use]
pub struct Value<const BITS: usize>(pub(crate) u128);

impl<const BITS: usize> Value<BITS> {
    /// Compile-time guard: associating with this constant fails
    /// the build if `BITS` exceeds the 128-bit backing store.
    const VALID: () = assert!(BITS <= 128);

    /// Bitmask of the low `BITS` bits, used to clamp every result to width.
    const MASK: u128 = match BITS {
        0 => 0,
        1..=127 => (1u128 << BITS) - 1,
        128.. => u128::MAX,
    };
}

/// Builds the five overflow-aware variants of a numeric op (`wrapping_*`,
/// `checked_*`, `overflowing_*`, `strict_*`, `saturating_*`) from a single
/// `u128::overflowing_*` primitive. All five bodies are identical for every
/// op; they differ only in which primitive is called, the panic message, and
/// the clamp value used on saturation (`MASK` for growing ops, `ZERO` for
/// shrinking ones like `sub`/`neg`).
macro_rules! overflowing_family {
    ($(
        $wrapping:ident / $checked:ident / $overflowing:ident / $strict:ident / $saturating:ident
            $(<$gen:ident: Into<u128>>)? ($($param:ident : $pty:ty),*)
            via $inherent:ident($($arg:expr),*) as $msg:literal, saturate = $sat:expr;
    )+) => {$(
        #[doc = concat!("Wrapping ", $msg, ".")]
        pub fn $wrapping$(<$gen: Into<u128>>)?(self, $($param: $pty),*) -> Self {
            Metrics::charge::<BITS>(Operation::$wrapping, Costs::get().$wrapping);
            Self(self.0.$inherent($($arg),*).0 & Self::MASK)
        }

        #[doc = concat!("Checked ", $msg, ". `None` if the result overflows `BITS` width.")]
        pub fn $checked$(<$gen: Into<u128>>)?(self, $($param: $pty),*) -> Option<Self> {
            Metrics::charge::<BITS>(Operation::$checked, Costs::get().$checked);
            let (raw, of) = self.0.$inherent($($arg),*);
            let masked = raw & Self::MASK;
            (!of && masked == raw).then_some(Self(masked))
        }

        #[doc = concat!("Overflowing ", $msg, ". Returns the wrapped result and whether it overflowed.")]
        pub fn $overflowing$(<$gen: Into<u128>>)?(self, $($param: $pty),*) -> (Self, bool) {
            Metrics::charge::<BITS>(Operation::$overflowing, Costs::get().$overflowing);
            let (raw, of) = self.0.$inherent($($arg),*);
            let masked = raw & Self::MASK;
            (Self(masked), of || masked != raw)
        }

        #[doc = concat!("Strict ", $msg, ". Panics if the result overflows `BITS` width.")]
        pub fn $strict$(<$gen: Into<u128>>)?(self, $($param: $pty),*) -> Self {
            Metrics::charge::<BITS>(Operation::$strict, Costs::get().$strict);
            let (raw, of) = self.0.$inherent($($arg),*);
            let masked = raw & Self::MASK;
            assert!(!of && masked == raw, concat!("attempt to ", $msg, " with overflow"));
            Self(masked)
        }

        #[doc = concat!("Saturating ", $msg, ". Clamps on overflow.")]
        pub fn $saturating$(<$gen: Into<u128>>)?(self, $($param: $pty),*) -> Self {
            Metrics::charge::<BITS>(Operation::$saturating, Costs::get().$saturating);
            let (raw, of) = self.0.$inherent($($arg),*);
            let masked = raw & Self::MASK;
            if of || masked != raw { $sat } else { Self(masked) }
        }
    )+};
}

/// Builds `wrapping_*`/`checked_*`/`overflowing_*`/`strict_*`/`saturating_*`
/// for a division-like op, where "overflow" is really divide-by-zero and the
/// primitive is the raw `/` or `%` operator rather than an `overflowing_*` fn.
macro_rules! division_family {
    ($(
        $wrapping:ident / $checked:ident / $overflowing:ident / $strict:ident $(/ $saturating:ident)? via $op:tt;
    )+) => {$(
        #[doc = concat!("Wrapping `", stringify!($op), "`. Panics on divide-by-zero.")]
        pub fn $wrapping<V: Into<u128>>(self, rhs: V) -> Self {
            Metrics::charge::<BITS>(Operation::$wrapping, Costs::get().$wrapping);
            Self((self.0 $op rhs.into()) & Self::MASK)
        }

        #[doc = concat!("Checked `", stringify!($op), "`. `None` on divide-by-zero.")]
        pub fn $checked<V: Into<u128>>(self, rhs: V) -> Option<Self> {
            Metrics::charge::<BITS>(Operation::$checked, Costs::get().$checked);
            let rhs = rhs.into();
            (rhs != 0).then(|| Self((self.0 $op rhs) & Self::MASK))
        }

        #[doc = concat!("Overflowing `", stringify!($op), "`. The flag is always `false`. Panics on divide-by-zero.")]
        pub fn $overflowing<V: Into<u128>>(self, rhs: V) -> (Self, bool) {
            Metrics::charge::<BITS>(Operation::$overflowing, Costs::get().$overflowing);
            (Self((self.0 $op rhs.into()) & Self::MASK), false)
        }

        #[doc = concat!("Strict `", stringify!($op), "`. Panics on divide-by-zero.")]
        pub fn $strict<V: Into<u128>>(self, rhs: V) -> Self {
            Metrics::charge::<BITS>(Operation::$strict, Costs::get().$strict);
            Self((self.0 $op rhs.into()) & Self::MASK)
        }

        $(
            #[doc = concat!("Saturating `", stringify!($op), "`. Cannot overflow, so equal to wrapping.")]
            pub fn $saturating<V: Into<u128>>(self, rhs: V) -> Self {
                Metrics::charge::<BITS>(Operation::$saturating, Costs::get().$saturating);
                Self((self.0 $op rhs.into()) & Self::MASK)
            }
        )?
    )+};
}

/// Builds `wrapping_*`/`checked_*`/`overflowing_*`/`strict_*` for a shift op
/// (no `saturating_*`: shifts don't have a meaningful saturation target).
/// `wrapping`/`overflowing` reduce the shift count modulo `BITS`; `checked`/
/// `strict` reject any count `>= BITS`.
macro_rules! shift_family {
    ($(
        $wrapping:ident / $checked:ident / $overflowing:ident / $strict:ident via $op:tt as $dir:literal;
    )+) => {$(
        #[doc = concat!("Wrapping ", $dir, " shift. Reduces the shift amount modulo `BITS`.")]
        pub fn $wrapping(self, rhs: u32) -> Self {
            Metrics::charge::<BITS>(Operation::$wrapping, Costs::get().$wrapping);
            Self((self.0 $op (rhs % (BITS as u32).max(1))) & Self::MASK)
        }

        #[doc = concat!("Checked ", $dir, " shift. `None` if `rhs >= BITS`.")]
        pub fn $checked(self, rhs: u32) -> Option<Self> {
            Metrics::charge::<BITS>(Operation::$checked, Costs::get().$checked);
            (rhs < BITS as u32).then(|| Self((self.0 $op rhs) & Self::MASK))
        }

        #[doc = concat!("Overflowing ", $dir, " shift. The flag is set if `rhs >= BITS`.")]
        pub fn $overflowing(self, rhs: u32) -> (Self, bool) {
            Metrics::charge::<BITS>(Operation::$overflowing, Costs::get().$overflowing);
            let masked = (self.0 $op (rhs % (BITS as u32).max(1))) & Self::MASK;
            (Self(masked), rhs >= BITS as u32)
        }

        #[doc = concat!("Strict ", $dir, " shift. Panics if `rhs >= BITS`.")]
        pub fn $strict(self, rhs: u32) -> Self {
            Metrics::charge::<BITS>(Operation::$strict, Costs::get().$strict);
            assert!(rhs < BITS as u32, concat!("attempt to shift ", $dir, " with overflow"));
            Self((self.0 $op rhs) & Self::MASK)
        }
    )+};
}

impl<const BITS: usize> Value<BITS> {
    /// Static area of one value: its `BITS` width, in the cost model's area units.
    pub const SIZE: f64 = BITS as f64;

    /// The zero value.
    pub const ZERO: Self = Self(0);

    /// Wraps a raw `u128` without masking. Caller must guarantee it already fits
    /// `BITS`; in most cases prefer [`Value::wrapping`] or [`Value::failing`].
    pub const fn new(value: u128) -> Self {
        debug_assert!(
            value & !Self::MASK == 0,
            "Value::new called with a value that does not fit BITS",
        );
        Self(value)
    }

    /// Constructs a value, truncating `value` to `BITS` width (i.e. modulo `2^BITS`).
    /// Use when an out-of-range input should silently wrap rather than fail.
    pub fn wrapping<V>(value: V) -> Self
    where
        V: Into<u128>,
    {
        let () = Self::VALID;
        let value = value.into();

        Self(value & Self::MASK)
    }

    /// Constructs a value, returning `None` if `value` does not fit `BITS` width.
    /// Use when an out-of-range input should fail rather than silently wrap.
    pub fn failing<V>(value: V) -> Option<Self>
    where
        V: Into<u128>,
    {
        let () = Self::VALID;
        let value = value.into();

        if value & !Self::MASK == 0 {
            Some(Self::new(value))
        } else {
            None
        }
    }

    overflowing_family! {
        wrapping_add / checked_add / overflowing_add / strict_add / saturating_add
            <V: Into<u128>>(rhs: V) via overflowing_add(rhs.into()) as "add", saturate = Self(Self::MASK);

        wrapping_sub / checked_sub / overflowing_sub / strict_sub / saturating_sub
            <V: Into<u128>>(rhs: V) via overflowing_sub(rhs.into()) as "subtract", saturate = Self::ZERO;

        wrapping_mul / checked_mul / overflowing_mul / strict_mul / saturating_mul
            <V: Into<u128>>(rhs: V) via overflowing_mul(rhs.into()) as "multiply", saturate = Self(Self::MASK);

        wrapping_pow / checked_pow / overflowing_pow / strict_pow / saturating_pow
            (exp: u32) via overflowing_pow(exp) as "pow", saturate = Self(Self::MASK);

        wrapping_neg / checked_neg / overflowing_neg / strict_neg / saturating_neg
            () via overflowing_neg() as "negate", saturate = Self::ZERO;
    }

    division_family! {
        wrapping_div / checked_div / overflowing_div / strict_div / saturating_div via /;
        wrapping_rem / checked_rem / overflowing_rem / strict_rem via %;
    }

    shift_family! {
        wrapping_shl / checked_shl / overflowing_shl / strict_shl via << as "left";
        wrapping_shr / checked_shr / overflowing_shr / strict_shr via >> as "right";
    }

    /// Bitwise AND within `BITS` width.
    #[allow(clippy::should_implement_trait)] // NOTE(fidelicura): Implementation is actively reused.
    pub fn bitand<V>(self, rhs: V) -> Self
    where
        V: Into<u128>,
    {
        Metrics::charge::<BITS>(Operation::bitand, Costs::get().bitand);
        Self(self.0 & rhs.into() & Self::MASK)
    }

    /// Bitwise OR within `BITS` width.
    #[allow(clippy::should_implement_trait)] // NOTE(fidelicura): Implementation is actively reused.
    pub fn bitor<V>(self, rhs: V) -> Self
    where
        V: Into<u128>,
    {
        Metrics::charge::<BITS>(Operation::bitor, Costs::get().bitor);
        Self((self.0 | rhs.into()) & Self::MASK)
    }

    /// Bitwise XOR within `BITS` width.
    #[allow(clippy::should_implement_trait)] // NOTE(fidelicura): Implementation is actively reused.
    pub fn bitxor<V>(self, rhs: V) -> Self
    where
        V: Into<u128>,
    {
        Metrics::charge::<BITS>(Operation::bitxor, Costs::get().bitxor);
        Self((self.0 ^ rhs.into()) & Self::MASK)
    }

    /// Bitwise NOT within `BITS` width.
    pub fn bitnot(self) -> Self {
        Metrics::charge::<BITS>(Operation::bitnot, Costs::get().bitnot);
        Self(!self.0 & Self::MASK)
    }

    /// Explicit re-typing to `NEW` bits: drops the high bits when narrowing,
    /// zero-extends when widening. The only sanctioned way to combine two
    /// `Value`s of different widths - every operator requires matching `BITS`
    /// on purpose, so a width change is always this explicit, priced step
    /// rather than something an operator does implicitly.
    ///
    /// Priced like every other method here: keyed by `BITS` (the source
    /// width), same convention as `shift_family!` keying by the shiftee
    /// rather than the shift amount's width.
    pub fn resize<const NEW: usize>(self) -> Value<NEW> {
        Metrics::charge::<BITS>(Operation::resize, Costs::get().resize);
        Value(self.0 & Value::<NEW>::MASK)
    }
}

/// `match` that also charges the cost of a multiplexer with one input per arm.
///
/// Behaves exactly like `match $selection { ... }`, but first records a `select`
/// cost proportional to the number of arms (see [`Metrics::select`]). Use it
/// wherever a selection in modelled hardware would be realised as a multiplexer.
#[macro_export]
macro_rules! select {
    ($selection:expr => { $($pattern:pat => $arm:expr),+ $(,)? }) => {{
        $crate::Metrics::select::<{ [$($crate::select!(@one $pattern)),+].len() }>();

        match $selection { $($pattern => $arm),+ }
    }};

    // Internal arm that maps each arm pattern to `()` so the surrounding
    // array's length counts the arms (i.e. the number of multiplexer inputs).
    (@one $_pat:pat) => { () };
}

impl<const BITS: usize> Default for Value<BITS> {
    #[inline(always)]
    fn default() -> Self {
        Self::ZERO
    }
}

// NOTE(fidelicura): Operator traits delegate to the inherent methods above, keeping
// the cost model in one place: the inherent op charges the read, `*Assign` adds an
// `assign` write, comparisons charge one `select`. These macros stamp out the
// boilerplate over `Value` and every supported unsigned-width right-hand side.

/// `Trait::method => inherent_op`, value-returning, over `Value` and each width.
macro_rules! binary_operation {
    ($($trait:ident::$method:ident => $inherent:ident),+ $(,)?) => {$(
        impl<const BITS: usize> ops::$trait<Value<BITS>> for Value<BITS> {
            type Output = Value<BITS>;
            fn $method(self, rhs: Value<BITS>) -> Self { self.$inherent(rhs.0) }
        }
        binary_operation!(@uint $trait::$method => $inherent; u8, u16, u32, u64);
    )+};
    (@uint $trait:ident::$method:ident => $inherent:ident; $($t:ty),+) => {$(
        impl<const BITS: usize> ops::$trait<$t> for Value<BITS> {
            type Output = Value<BITS>;
            fn $method(self, rhs: $t) -> Self { self.$inherent(rhs) }
        }
    )+};
}

binary_operation! {
    Add::add => wrapping_add,
    BitAnd::bitand => bitand,
    BitOr::bitor => bitor,
    BitXor::bitxor => bitxor,
}

impl<const BITS: usize> ops::Not for Value<BITS> {
    type Output = Value<BITS>;

    fn not(self) -> Self::Output {
        self.bitnot()
    }
}

/// Like [`binary_operation`] but for shifts, whose inherent op takes the count as `u32`.
macro_rules! shift_operation {
    ($($trait:ident::$method:ident => $inherent:ident),+ $(,)?) => {$(
        impl<const BITS: usize> ops::$trait<Value<BITS>> for Value<BITS> {
            type Output = Value<BITS>;
            fn $method(self, rhs: Value<BITS>) -> Self { self.$inherent(rhs.0 as u32) }
        }
        shift_operation!(@uint $trait::$method => $inherent; u8, u16, u32, u64);
    )+};
    (@uint $trait:ident::$method:ident => $inherent:ident; $($t:ty),+) => {$(
        impl<const BITS: usize> ops::$trait<$t> for Value<BITS> {
            type Output = Value<BITS>;
            #[allow(clippy::unnecessary_cast)]
            fn $method(self, rhs: $t) -> Self { self.$inherent(rhs as u32) }
        }
    )+};
}

shift_operation! {
    Shl::shl => wrapping_shl,
    Shr::shr => wrapping_shr,
}

/// `Trait::method => inherent_op` for a `*_assign`: the inherent op charges the
/// read, then `assign` charges the write back. `$($cast)*` is `as u32` for shifts.
macro_rules! assign_operation {
    ($($trait:ident::$method:ident => $inherent:ident $(as $cast:ty)?),+ $(,)?) => {$(
        impl<const BITS: usize> ops::$trait<Value<BITS>> for Value<BITS> {
            fn $method(&mut self, rhs: Value<BITS>) {
                Metrics::charge::<BITS>(Operation::assign, Costs::get().assign);
                *self = self.$inherent(rhs.0 $(as $cast)?);
            }
        }
    )+};
}

assign_operation! {
    AddAssign::add_assign => wrapping_add,
    BitAndAssign::bitand_assign => bitand,
    BitOrAssign::bitor_assign => bitor,
    BitXorAssign::bitxor_assign => bitxor,
    ShlAssign::shl_assign => wrapping_shl as u32,
    ShrAssign::shr_assign => wrapping_shr as u32,
}

impl<const BITS: usize> cmp::PartialEq for Value<BITS> {
    fn eq(&self, rhs: &Self) -> bool {
        Metrics::select::<2>();
        self.0 == rhs.0
    }
}

impl<const BITS: usize> cmp::Eq for Value<BITS> {}

impl<const BITS: usize> cmp::Ord for Value<BITS> {
    fn cmp(&self, rhs: &Self) -> Ordering {
        Metrics::select::<2>();
        self.0.cmp(&rhs.0)
    }
}

impl<const BITS: usize> cmp::PartialOrd for Value<BITS> {
    fn partial_cmp(&self, rhs: &Self) -> Option<Ordering> {
        Some(self.cmp(rhs))
    }
}

/// `PartialEq`/`PartialOrd` against each unsigned width, each charging a `select`.
macro_rules! compare_operation {
    ($($t:ty),+ $(,)?) => {$(
        impl<const BITS: usize> cmp::PartialEq<$t> for Value<BITS> {
            fn eq(&self, rhs: &$t) -> bool {
                Metrics::select::<2>();
                self.0 == *rhs as u128
            }
        }
        impl<const BITS: usize> cmp::PartialOrd<$t> for Value<BITS> {
            fn partial_cmp(&self, rhs: &$t) -> Option<Ordering> {
                Metrics::select::<2>();
                self.0.partial_cmp(&(*rhs as u128))
            }
        }
    )+};
}

compare_operation!(u8, u16, u32, u64);

////////////////////////////////////////////////////////////////////////////////////////////////////
// }} Value
////////////////////////////////////////////////////////////////////////////////////////////////////

#[cfg(test)]
mod tests {
    use super::*;
    use crate::observability::Cost;

    /// All arithmetic is checked against this 8-bit width, so `MASK == 255`
    /// and any result `>= 256` exercises the overflow path.
    type V = Value<8>;

    /// These tests exercise arithmetic, not the cost model, so price every op at
    /// zero. An unset cost panics when charged (see [`Costs`]), and the table is
    /// a thread-local shared across tests, so each charging test sets it.
    fn free_costs() {
        Costs::set(Costs::free());
    }

    #[test]
    fn add() {
        free_costs();
        let a = V::wrapping(200u16); // 200 + 100 = 300 overflows, 200 + 10 = 210 fits.

        assert_eq!(a.wrapping_add(10u16), 210u32);
        assert_eq!(a.wrapping_add(100u16), 44u32); // 300 mod 256.

        assert_eq!(a.checked_add(50u16), Some(V::wrapping(250u16)));
        assert_eq!(a.checked_add(100u16), None);

        assert_eq!(a.overflowing_add(1u16), (V::wrapping(201u16), false));
        assert_eq!(a.overflowing_add(100u16), (V::wrapping(44u16), true));

        assert_eq!(a.saturating_add(10u16), 210u32);
        assert_eq!(a.saturating_add(100u16), 255u32);

        assert_eq!(a.strict_add(10u16), 210u32);
    }

    #[test]
    #[should_panic(expected = "attempt to add with overflow")]
    fn add_strict_panics_on_overflow() {
        free_costs();
        let _ = V::wrapping(255u16).strict_add(1u16);
    }

    #[test]
    #[should_panic(expected = "does not fit BITS")]
    fn new_debug_asserts_value_fits() {
        let _ = V::new(256); // 256 sets a bit above 8; debug_assert fires in test builds.
    }

    #[test]
    fn sub() {
        free_costs();
        let a = V::wrapping(200u16);

        assert_eq!(a.wrapping_sub(10u16), 190u32);
        assert_eq!(V::ZERO.wrapping_sub(1u16), 255u32); // Underflow wraps.

        assert_eq!(a.checked_sub(1u16), Some(V::wrapping(199u16)));
        assert_eq!(V::ZERO.checked_sub(1u16), None);

        assert_eq!(a.overflowing_sub(1u16), (V::wrapping(199u16), false));
        assert_eq!(V::ZERO.overflowing_sub(1u16), (V::wrapping(255u16), true));

        assert_eq!(a.saturating_sub(10u16), 190u32);
        assert_eq!(V::ZERO.saturating_sub(1u16), 0u32);

        assert_eq!(a.strict_sub(10u16), 190u32);
    }

    #[test]
    fn mul() {
        free_costs();
        assert_eq!(V::wrapping(20u16).wrapping_mul(20u16), 400u32 & 255); // 400 mod 256 = 144.

        assert_eq!(
            V::wrapping(10u16).checked_mul(2u16),
            Some(V::wrapping(20u16))
        );
        assert_eq!(V::wrapping(200u16).checked_mul(2u16), None);

        assert_eq!(
            V::wrapping(2u16).overflowing_mul(3u16),
            (V::wrapping(6u16), false)
        );

        assert_eq!(V::wrapping(10u16).saturating_mul(10u16), 100u32);
        assert_eq!(V::wrapping(20u16).saturating_mul(20u16), 255u32);

        assert_eq!(V::wrapping(10u16).strict_mul(10u16), 100u32);
    }

    #[test]
    fn pow() {
        free_costs();
        assert_eq!(V::wrapping(2u16).wrapping_pow(3), 8u32);
        assert_eq!(V::wrapping(2u16).wrapping_pow(8), 0u32); // 256 mod 256.

        assert_eq!(V::wrapping(2u16).checked_pow(3), Some(V::wrapping(8u16)));
        assert_eq!(V::wrapping(2u16).checked_pow(8), None);

        assert_eq!(
            V::wrapping(2u16).overflowing_pow(3),
            (V::wrapping(8u16), false)
        );
        assert!(V::wrapping(200u16).overflowing_pow(2).1);

        assert_eq!(V::wrapping(2u16).saturating_pow(3), 8u32);
        assert_eq!(V::wrapping(2u16).saturating_pow(8), 255u32);

        assert_eq!(V::wrapping(2u16).strict_pow(3), 8u32);
    }

    #[test]
    fn div() {
        free_costs();
        let a = V::wrapping(200u16); // 200 / 7 = 28

        // Div never overflows, so wrapping/saturating/overflowing/strict all agree.
        assert_eq!(a.wrapping_div(7u16), 28u32);
        assert_eq!(a.saturating_div(7u16), 28u32);
        assert_eq!(a.strict_div(7u16), 28u32);
        assert_eq!(a.overflowing_div(7u16), (V::wrapping(28u16), false));
        assert_eq!(a.checked_div(7u16), Some(V::wrapping(28u16)));
        assert_eq!(a.checked_div(0u16), None); // Only divide-by-zero is rejected.
    }

    #[test]
    fn rem() {
        free_costs();
        let a = V::wrapping(200u16); // 200 % 7 = 4.

        assert_eq!(a.wrapping_rem(7u16), 4u32);
        assert_eq!(a.strict_rem(7u16), 4u32);
        assert_eq!(a.overflowing_rem(7u16), (V::wrapping(4u16), false));
        assert_eq!(a.checked_rem(7u16), Some(V::wrapping(4u16)));
        assert_eq!(a.checked_rem(0u16), None);
    }

    #[test]
    fn neg() {
        free_costs();
        assert_eq!(V::wrapping(1u16).wrapping_neg(), 255u32); // -1 mod 256.
        assert_eq!(V::ZERO.wrapping_neg(), V::ZERO);

        assert_eq!(V::ZERO.checked_neg(), Some(V::ZERO));
        assert_eq!(V::wrapping(1u16).checked_neg(), None); // Any non-zero overflows.

        assert_eq!(V::ZERO.overflowing_neg(), (V::ZERO, false));
        assert!(V::wrapping(1u16).overflowing_neg().1);

        assert_eq!(V::wrapping(1u16).saturating_neg(), 0u32); // Min unsigned is zero.

        assert_eq!(V::ZERO.strict_neg(), V::ZERO);
    }

    #[test]
    fn sh() {
        free_costs();
        assert_eq!(V::wrapping(1u16).wrapping_shl(3), 8u32);
        assert_eq!(V::wrapping(255u16).wrapping_shr(4), 15u32);

        assert_eq!(V::wrapping(1u16).checked_shl(3), Some(V::wrapping(8u16)));
        assert_eq!(V::wrapping(1u16).checked_shl(8), None); // Shift amount >= BITS.
        assert_eq!(V::wrapping(8u16).checked_shr(3), Some(V::wrapping(1u16)));
        assert_eq!(V::wrapping(8u16).checked_shr(8), None);

        // Overflowing reduces the shift modulo BITS and flags rhs >= BITS.
        assert_eq!(
            V::wrapping(1u16).overflowing_shl(8),
            (V::wrapping(1u16), true)
        );
        assert_eq!(
            V::wrapping(8u16).overflowing_shr(8),
            (V::wrapping(8u16), true)
        );

        assert_eq!(V::wrapping(1u16).strict_shl(3), 8u32);
        assert_eq!(V::wrapping(8u16).strict_shr(3), 1u32);
    }

    #[test]
    fn bit_ops() {
        free_costs();
        assert_eq!(V::wrapping(0b1100u16).bitand(0b1010u16), 0b1000u32);
        assert_eq!(V::wrapping(0b1100u16).bitor(0b1010u16), 0b1110u32);
        assert_eq!(V::wrapping(0b1100u16).bitxor(0b1010u16), 0b0110u32);
        assert_eq!(V::ZERO.bitnot(), 255u32);
    }

    #[test]
    fn constructors() {
        free_costs();
        assert_eq!(V::new(5), 5u32);
        assert_eq!(V::default(), V::ZERO);
        assert_eq!(V::failing(5u16), Some(V::wrapping(5u16)));
        assert_eq!(V::failing(256u16), None); // Does not fit 8 bits.
    }

    #[test]
    fn operator_add_across_widths() {
        free_costs();
        let a = V::wrapping(12u16);

        assert_eq!(a + V::wrapping(10u16), 22u32);
        assert_eq!(a + 1u8, 13u32);
        assert_eq!(a + 1u16, 13u32);
        assert_eq!(a + 1u32, 13u32);
        assert_eq!(a + 1u64, 13u32);

        let mut c = a;
        c += V::wrapping(10u16);
        assert_eq!(c, 22u32);
    }

    #[test]
    fn operator_bit_ops_across_widths() {
        free_costs();
        let a = V::wrapping(12u16);
        let b = V::wrapping(10u16);

        for (and, or, xor) in [
            (a & b, a | b, a ^ b),
            (a & 10u8, a | 10u8, a ^ 10u8),
            (a & 10u16, a | 10u16, a ^ 10u16),
            (a & 10u32, a | 10u32, a ^ 10u32),
            (a & 10u64, a | 10u64, a ^ 10u64),
        ] {
            assert_eq!(and, 12u32 & 10);
            assert_eq!(or, 12u32 | 10);
            assert_eq!(xor, 12u32 ^ 10);
        }

        assert_eq!(!V::ZERO, 255u32);
    }

    #[test]
    fn operator_shifts_across_widths() {
        free_costs();
        let a = V::wrapping(12u16);
        let one = V::wrapping(1u16);

        assert_eq!(a >> V::wrapping(2u16), 3u32);
        assert_eq!(one << V::wrapping(2u16), 4u32);

        for (shr, shl) in [
            (a >> 1u8, one << 1u8),
            (a >> 1u16, one << 1u16),
            (a >> 1u32, one << 1u32),
            (a >> 1u64, one << 1u64),
        ] {
            assert_eq!(shr, 6u32);
            assert_eq!(shl, 2u32);
        }
    }

    #[test]
    fn operator_compare_across_widths() {
        free_costs();
        let a = V::wrapping(12u16);

        assert!(a == 12u8 && a > 1u8);
        assert!(a == 12u16 && a > 1u16);
        assert!(a == 12u32 && a > 1u32);
        assert!(a == 12u64 && a > 1u64);
    }

    #[test]
    fn compare_charges_one_select_each() {
        Costs::set(Costs {
            select: Cost {
                switching_energy: Some(1.0),
                ..Cost::free()
            },
            ..Costs::free()
        });
        Metrics::reset();

        let (a, b) = (V::wrapping(1u16), V::wrapping(2u16));
        let _ = a == b; // Value/Value eq.
        let _ = a < b; // Value/Value ord (via partial_cmp -> cmp, charged once).
        let _ = a == 1u8; // Value/uN eq.
        let _ = a < 9u32; // Value/uN ord.

        assert_eq!(Metrics::reset().energy, 4.0); // 4 compares * 1 select.
        Costs::set(Costs::default());
    }

    #[test]
    fn add_assign_charges_read_plus_write() {
        Costs::set(Costs {
            wrapping_add: Cost {
                switching_energy: Some(2.0),
                ..Cost::free()
            },
            assign: Cost {
                switching_energy: Some(5.0),
                ..Cost::free()
            },
            ..Costs::free()
        });
        Metrics::reset();

        let mut a = V::wrapping(1u16);
        a += V::wrapping(1u16);

        assert_eq!(Metrics::reset().energy, 7.0); // 2 read + 5 write.
        Costs::set(Costs::default());
    }

    #[test]
    fn every_assign_charges_one_write() {
        // Price reads free and the write at 1.0, so the total counts the writes.
        Costs::set(Costs {
            assign: Cost {
                switching_energy: Some(1.0),
                ..Cost::free()
            },
            ..Costs::free()
        });
        Metrics::reset();

        let mut a = V::wrapping(0b1100u16);
        a += V::wrapping(1u16);
        a &= V::wrapping(0b1010u16);
        a |= V::wrapping(0b0001u16);
        a ^= V::wrapping(0b0010u16);
        a <<= V::wrapping(1u16);
        a >>= V::wrapping(1u16);

        assert_eq!(Metrics::reset().energy, 6.0); // 6 assigns * 1 write.
        Costs::set(Costs::default());
    }

    #[test]
    fn select() {
        free_costs();
        let pick = |selection: u8| select!(selection => { 0 => 10, 1 => 20, _ => 30 });
        assert_eq!(pick(0), 10);
        assert_eq!(pick(1), 20);
        assert_eq!(pick(2), 30);
    }
}
