/*
 * Copyright 2020 Google LLC
 *
 * Licensed under the Apache License, Version 2.0 (the "License");
 * you may not use this file except in compliance with the License.
 * You may obtain a copy of the License at
 *
 *     http://www.apache.org/licenses/LICENSE-2.0
 *
 * Unless required by applicable law or agreed to in writing, software
 * distributed under the License is distributed on an "AS IS" BASIS,
 * WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
 * See the License for the specific language governing permissions and
 * limitations under the License.
 */

use once_cell::sync::Lazy;
use prometheus::{Histogram, IntCounter, IntGauge, Opts};

use crate::metrics::{histogram_opts, register};

const SUBSYSTEM: &str = "session";

pub(crate) static ACTIVE_SESSIONS: Lazy<IntGauge> = Lazy::new(|| {
    register(
        IntGauge::with_opts(
            Opts::new("active", "number of sessions currently active").subsystem(SUBSYSTEM),
        )
        .unwrap(),
    )
});

pub(crate) static TOTAL_SESSIONS: Lazy<IntCounter> = Lazy::new(|| {
    register(
        IntCounter::with_opts(
            Opts::new("total", "total number of established sessions").subsystem(SUBSYSTEM),
        )
        .unwrap(),
    )
});

pub(crate) static DURATION_SECS: Lazy<Histogram> = Lazy::new(|| {
    register(
        Histogram::with_opts(histogram_opts(
            "duration_secs",
            SUBSYSTEM,
            "duration of sessions",
            vec![
                1f64, 5f64, 10f64, 25f64, 60f64, 300f64, 900f64, 1800f64, 3600f64,
            ],
        ))
        .unwrap(),
    )
});
