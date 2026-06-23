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

use ramus::Config;
use std::error::Error;

/// Default simulation config: 1024-record warmup, uncharged, no speculation.
const DEFAULT: &str = include_str!("../config.json");

/// Load the simulation config from `path`, or the built-in default when `path` is `None`.
pub fn load(path: &Option<String>) -> Result<Config, Box<dyn Error>> {
    let payload = if let Some(path) = path {
        std::fs::read_to_string(path).map_err(|error| format!("{path}: {error}"))?
    } else {
        DEFAULT.to_string()
    };
    Ok(serde_json::from_str(&payload)?)
}
