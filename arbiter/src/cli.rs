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

use clap::Parser;

/// Trace-driven branch-predictor evaluation: run every `--predictors` selection
/// over every `--traces` file, printing one JSON report per (trace, predictor) pair.
#[derive(Parser)]
pub struct Args {
    /// Trace files to sweep - CBP-NG 2026 Gzipped. Directories expand to their contained files.
    #[arg(long, required = true, num_args = 1..)]
    pub traces: Vec<String>,

    /// Predictor names to run - see the error message for the full registry.
    #[arg(long, required = true, num_args = 1..)]
    pub predictors: Vec<String>,

    /// Cost table JSON path, overriding the built-in SkyWater-derived table.
    #[arg(long)]
    pub costs: Option<String>,

    /// Simulation config JSON path, overriding the built-in default.
    #[arg(long)]
    pub config: Option<String>,
}
