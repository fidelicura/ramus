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

use ramus::{Config, Metrics};
use serde::Serialize;

/// One predictor's run over one trace, raw totals plus the ratios derived from them.
#[derive(Serialize)]
pub struct Report<'a> {
    pub trace_path: &'a str,
    pub predictor_name: &'a str,
    pub configured_warmup: u64,
    pub configured_speculations: u64,
    pub total_predictions: u64,
    pub total_hits: u64,
    pub total_misses: u64,
    pub total_operations: u64,
    pub total_energy: f64,
    pub total_leakage: f64,
    pub total_space: f64,
    pub total_time: f64,
    pub hit_rate: f64,
    pub miss_rate: f64,
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
    pub fn new(trace: &'a str, predictor: &'a str, metric: Metrics, config: Config) -> Self {
        let misses = metric.predictions - metric.hits;

        Self {
            trace_path: trace,
            predictor_name: predictor,
            configured_warmup: config.warmup,
            configured_speculations: config.speculations,
            total_predictions: metric.predictions,
            total_hits: metric.hits,
            total_misses: misses,
            total_operations: metric.operations,
            total_energy: metric.energy,
            total_leakage: metric.leakage,
            total_space: metric.space,
            total_time: metric.time,
            hit_rate: ratio(metric.hits as f64, metric.predictions as f64),
            miss_rate: ratio(misses as f64, metric.predictions as f64),
            energy_per_prediction: ratio(metric.energy, metric.predictions as f64),
            energy_per_operation: ratio(metric.energy, metric.operations as f64),
            energy_per_hit: ratio(metric.energy, metric.hits as f64),
            energy_per_miss: ratio(metric.energy, misses as f64),
            energy_per_area: ratio(metric.energy, metric.space),
        }
    }
}
