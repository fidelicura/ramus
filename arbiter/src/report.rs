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

use comfy_table::Table;
use ramus::{Config, Metrics};
use serde::Serialize;

/// One predictor's run over one trace, raw totals plus the ratios derived from them.
#[derive(Serialize)]
pub struct Report<'a> {
    pub run_identifier: String,
    pub trace_path: &'a str,
    pub predictor_name: &'a str,
    pub configured_warmup: u64,
    pub configured_speculations: u64,
    pub total_predictions: u64,
    pub total_hits: u64,
    pub total_misses: u64,
    pub total_direction_predictions: u64,
    pub total_direction_hits: u64,
    pub total_direction_misses: u64,
    pub total_target_predictions: u64,
    pub total_target_hits: u64,
    pub total_target_misses: u64,
    pub total_operations: u64,
    pub total_energy: f64,
    pub total_leakage: f64,
    pub total_space: f64,
    pub total_time: u64,
    pub hit_rate: f64,
    pub miss_rate: f64,
    pub direction_accuracy: f64,
    pub target_accuracy: f64,
    pub energy_per_prediction: f64,
    pub energy_per_operation: f64,
    pub energy_per_hit: f64,
    pub energy_per_miss: f64,
    pub energy_per_area: f64,
}

/// Divide guarding a zero denominator, which yields `0.0` rather than NaN.
fn ratio(numerator: f64, denominator: f64) -> f64 {
    if denominator == 0.0 {
        0.0
    } else {
        numerator / denominator
    }
}

impl<'a> Report<'a> {
    /// Build one report from `trace` and `predictor`'s names and its `metric`.
    pub fn new(
        trace: &'a str,
        predictor: &'a str,
        metric: Metrics,
        config: Config,
        name: Option<&str>,
    ) -> Self {
        let misses = metric.total_predictions - metric.total_hits;
        let direction_misses = metric.direction_predictions - metric.direction_hits;
        let target_misses = metric.target_predictions - metric.target_hits;

        Self {
            run_identifier: name.map_or_else(
                || petname::petname(3, "-").expect("builtin word lists should not be empty"),
                str::to_owned,
            ),
            trace_path: trace,
            predictor_name: predictor,
            configured_warmup: config.warmup,
            configured_speculations: config.speculations,
            total_predictions: metric.total_predictions,
            total_hits: metric.total_hits,
            total_misses: misses,
            total_direction_predictions: metric.direction_predictions,
            total_direction_hits: metric.direction_hits,
            total_direction_misses: direction_misses,
            total_target_predictions: metric.target_predictions,
            total_target_hits: metric.target_hits,
            total_target_misses: target_misses,
            total_operations: metric.total_operations,
            total_energy: metric.total_energy,
            total_leakage: metric.total_leakage,
            total_space: metric.total_space,
            total_time: metric.total_time,
            hit_rate: ratio(metric.total_hits as f64, metric.total_predictions as f64),
            miss_rate: ratio(misses as f64, metric.total_predictions as f64),
            direction_accuracy: ratio(
                metric.direction_hits as f64,
                metric.direction_predictions as f64,
            ),
            target_accuracy: ratio(metric.target_hits as f64, metric.target_predictions as f64),
            energy_per_prediction: ratio(metric.total_energy, metric.total_predictions as f64),
            energy_per_operation: ratio(metric.total_energy, metric.total_operations as f64),
            energy_per_hit: ratio(metric.total_energy, metric.total_hits as f64),
            energy_per_miss: ratio(metric.total_energy, misses as f64),
            energy_per_area: ratio(metric.total_energy, metric.total_space),
        }
    }
}

pub fn format_ascii(reports: &[Report<'_>]) -> String {
    if reports.is_empty() {
        return "No reports.".to_owned();
    }

    let rows = reports.iter().map(ascii_rows).collect::<Vec<_>>();
    let mut table = Table::new();
    table.set_header(
        std::iter::once("Metric".to_owned())
            .chain((1..=reports.len()).map(|run| format!("Run {run}")))
            .collect::<Vec<_>>(),
    );
    for (row, (label, _)) in rows[0].iter().enumerate() {
        table.add_row(
            std::iter::once((*label).to_owned())
                .chain(rows.iter().map(|run| run[row].1.clone()))
                .collect::<Vec<_>>(),
        );
    }
    table.to_string()
}

fn ascii_rows(report: &Report<'_>) -> Vec<(&'static str, String)> {
    vec![
        ("Run identifier", report.run_identifier.clone()),
        ("Trace path", report.trace_path.to_owned()),
        ("Predictor name", report.predictor_name.to_owned()),
        ("Configured warmup", report.configured_warmup.to_string()),
        (
            "Configured speculations",
            report.configured_speculations.to_string(),
        ),
        ("Total predictions", report.total_predictions.to_string()),
        ("Total hits", report.total_hits.to_string()),
        ("Total misses", report.total_misses.to_string()),
        ("Hit rate", format!("{:.2}%", report.hit_rate * 100.0)),
        ("Miss rate", format!("{:.2}%", report.miss_rate * 100.0)),
        (
            "Direction predictions",
            report.total_direction_predictions.to_string(),
        ),
        ("Direction hits", report.total_direction_hits.to_string()),
        (
            "Direction misses",
            report.total_direction_misses.to_string(),
        ),
        (
            "Direction accuracy",
            format!("{:.2}%", report.direction_accuracy * 100.0),
        ),
        (
            "Target predictions",
            report.total_target_predictions.to_string(),
        ),
        ("Target hits", report.total_target_hits.to_string()),
        ("Target misses", report.total_target_misses.to_string()),
        (
            "Target accuracy",
            format!("{:.2}%", report.target_accuracy * 100.0),
        ),
        ("Total operations", report.total_operations.to_string()),
        ("Total energy", format!("{:.6}", report.total_energy)),
        ("Total leakage", format!("{:.6}", report.total_leakage)),
        ("Total area", format!("{:.6}", report.total_space)),
        ("Total time", format!("{:.6}", report.total_time)),
        (
            "Energy/prediction",
            format!("{:.6}", report.energy_per_prediction),
        ),
        (
            "Energy/operation",
            format!("{:.6}", report.energy_per_operation),
        ),
        ("Energy/hit", format!("{:.6}", report.energy_per_hit)),
        ("Energy/miss", format!("{:.6}", report.energy_per_miss)),
        ("Energy/area", format!("{:.6}", report.energy_per_area)),
    ]
}
