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

use ramus::Predictor;

/// Arbiter traces carry 64-bit addresses; every predictor here is 64-bit.
pub type Address = ramus::Address<64>;

/// Registers each predictor module and builds [`ALL`] - predictors registry.
///
/// Each entry pairs a name with a builder that boxes a fresh predictor,
/// so the whole set can be fed to `Simulator::run_*` as trait objects.
macro_rules! predictors {
    ($($name:literal => $module:ident :: $ty:ident),+ $(,)?) => {
        $( mod $module; )+

        pub const ALL: &[(&str, fn() -> Box<dyn Predictor<64> + Send>)] = &[
            $( ($name, || Box::new($module::$ty::default())) ),+
        ];
    };
}

// XXX: Predictor name should not contain symbols that CLI
// interprets as delimiter between two different arguments.
predictors! {
    "smith_bimodal" => smith_bimodal::SmithBimodal,
}
