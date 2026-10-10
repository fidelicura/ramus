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

use clap::{Parser, ValueEnum};

/// Trace-driven branch-predictor evaluation: run every `--predictors` selection
/// over every `--traces` file for each `--warmups` value, printing one report
/// per (trace, warmup, predictor) combination.
#[derive(Parser)]
pub struct Arguments {
    /// Trace files to sweep - CBP-NG 2026 Gzipped.
    ///
    /// Directories expand to their contained files.
    #[arg(long, required = true, num_args = 1..)]
    pub traces: Vec<String>,

    /// Predictor names to run - see the error message for the full registry.
    #[arg(long, required = true, num_args = 1..)]
    pub predictors: Vec<String>,

    /// Cost table JSON path, overriding the built-in SkyWater-derived table.
    #[arg(long)]
    pub costs: Option<String>,

    /// Comma-separated warmup counts; each count runs a separate simulation.
    #[arg(long, value_delimiter = ',', default_value = "0")]
    pub warmups: Vec<u64>,

    /// Maximum number of predictions allowed to remain unresolved at once.
    #[arg(long, default_value_t = 0)]
    pub speculations: u64,

    /// Run identifier to use in every report, overriding the generated petname.
    #[arg(long)]
    pub name: Option<String>,

    /// Output format to use in every report.
    #[arg(long, value_enum, default_value = "ascii")]
    pub format: Format,
}

#[derive(Clone, Copy, ValueEnum)]
pub enum Format {
    Json,
    Ascii,
}

#[cfg(test)]
mod tests {
    use super::Arguments;
    use clap::Parser;

    #[test]
    fn parses_comma_separated_warmups() {
        let arguments = Arguments::try_parse_from([
            "arbiter",
            "--traces",
            "trace.gz",
            "--predictors",
            "bimodal",
            "--warmups",
            "0,16,128",
        ])
        .unwrap();

        assert_eq!(arguments.warmups, vec![0, 16, 128]);
    }
}
