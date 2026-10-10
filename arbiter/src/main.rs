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

mod cli;
mod costs;
mod predictors;
mod report;
mod trace;

use clap::Parser;
use cli::{Arguments, Format};
use ramus::{Config, Simulator};
use report::Report;
use std::error::Error;
use std::fs;
use trace::Trace;

fn main() -> Result<(), Box<dyn Error>> {
    let arguments = Arguments::parse();

    // Format all available predictor names.
    let available = predictors::ALL
        .iter()
        .map(|(name, _)| name)
        .map(ToString::to_string)
        .collect::<Vec<String>>()
        .join(", ");

    // Verify requested predictors to simulate.
    let mut selected = Vec::new();
    for name in &arguments.predictors {
        match predictors::ALL.iter().find(|entry| entry.0 == name) {
            Some(entry) => selected.push(entry),
            None => {
                return Err(format!("unknown predictor '{name}', available: {available}").into());
            }
        }
    }

    // Setup hardware costs that simulator will use.
    let costs = costs::load(&arguments.costs)?;

    // Expand any directories with traces into their contained files.
    let mut paths = Vec::new();
    for path in arguments.traces {
        if fs::metadata(&path)?.is_dir() {
            let entries = fs::read_dir(&path)?
                .filter_map(Result::ok)
                .map(|entry| entry.path())
                .filter(|entry| entry.is_file())
                .map(|entry| entry.to_string_lossy().into_owned())
                .collect::<Vec<String>>();
            paths.extend(entries);
        } else {
            paths.push(path);
        }
    }

    // Sweep every warmup and selected predictor over every requested trace.
    let mut reports = Vec::new();
    for path in &paths {
        // Read and parse the trace from user path.
        let trace = Trace::open(path).map_err(|error| format!("{path}: {error}"))?;

        for &warmup in &arguments.warmups {
            let config = Config {
                warmup,
                speculations: arguments.speculations,
            };

            // Heapify with `Box` one fresh predictor per user selection and warmup.
            let predictors = selected.iter().map(|(_, build)| build()).collect();
            let metrics =
                Simulator::run_many_predictors(trace.clone(), predictors, costs, config);

            // Build one report per predictor, in order of user selection.
            for (index, (name, _)) in selected.iter().enumerate() {
                let report = Report::new(
                    path,
                    name,
                    metrics[index],
                    config,
                    arguments.name.as_deref(),
                );
                reports.push(report);
            }
        }
    }

    // Print reports in the requested format.
    match arguments.format {
        Format::Json => println!("{}", serde_json::to_string_pretty(&reports)?),
        Format::Ascii => println!("{}", report::format_ascii(&reports)),
    }

    Ok(())
}
