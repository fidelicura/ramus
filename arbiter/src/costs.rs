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

use ramus::Costs;
use std::error::Error;

/// Hardware costs, sourced from the SkyWater's `sky130_fd_sc_hd` standard
/// cell library, drive strength X1, typical corner (`tt_025C_1v80`: 25C,
/// 1.8V). Each cost is that cell's own Liberty-reported `area` in squared
/// micrometers; `cell_leakage_power`, average |`internal_power`| over
/// its output pin's rise and fall tables, and average `cell_rise` and
/// `cell_fall` delay, all in nanowatts and nanoseconds respectively.
/// Where no cell was pulled for an operation, its cost is approximated
/// from the closest pulled cell instead of a separate synthesis run.
///
/// See <https://github.com/google/skywater-pdk-libs-sky130_fd_sc_hd>.
const DEFAULT: &str = include_str!("../costs.json");

/// Load the cost table from `path`, or the built-in default when `path` is `None`.
pub fn load(path: &Option<String>) -> Result<Costs, Box<dyn Error>> {
    let payload = if let Some(path) = path {
        std::fs::read_to_string(path).map_err(|error| format!("{path}: {error}"))?
    } else {
        DEFAULT.to_string()
    };
    Ok(serde_json::from_str(&payload)?)
}
