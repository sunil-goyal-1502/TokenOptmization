//! Process-wide compile metrics (for `/v1/metrics` and observability).

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::OnceLock;

use serde::{Deserialize, Serialize};

static COMPILE_REQUESTS: AtomicU64 = AtomicU64::new(0);
static COMPILE_ERRORS: AtomicU64 = AtomicU64::new(0);
static DURATION_MS_TOTAL: AtomicU64 = AtomicU64::new(0);
static DURATION_MS_MAX: AtomicU64 = AtomicU64::new(0);
static LAST_DURATION_MS: AtomicU64 = AtomicU64::new(0);
static REHYDRATE_REQUESTS: AtomicU64 = AtomicU64::new(0);

static LATENCY_BUCKET_MS: OnceLock<[AtomicU64; 6]> = OnceLock::new();

fn buckets() -> &'static [AtomicU64; 6] {
    LATENCY_BUCKET_MS.get_or_init(|| {
        [
            AtomicU64::new(0),
            AtomicU64::new(0),
            AtomicU64::new(0),
            AtomicU64::new(0),
            AtomicU64::new(0),
            AtomicU64::new(0),
        ]
    })
}

/// Histogram buckets: <=1ms, <=5ms, <=10ms, <=50ms, <=200ms, >200ms
fn record_latency_bucket(ms: u64) {
    let b = buckets();
    let idx = if ms <= 1 {
        0
    } else if ms <= 5 {
        1
    } else if ms <= 10 {
        2
    } else if ms <= 50 {
        3
    } else if ms <= 200 {
        4
    } else {
        5
    };
    b[idx].fetch_add(1, Ordering::Relaxed);
}

pub fn record_compile_success(duration_ms: u64) {
    COMPILE_REQUESTS.fetch_add(1, Ordering::Relaxed);
    DURATION_MS_TOTAL.fetch_add(duration_ms, Ordering::Relaxed);
    LAST_DURATION_MS.store(duration_ms, Ordering::Relaxed);
    record_latency_bucket(duration_ms);
    loop {
        let current = DURATION_MS_MAX.load(Ordering::Relaxed);
        if duration_ms <= current {
            break;
        }
        if DURATION_MS_MAX
            .compare_exchange(current, duration_ms, Ordering::Relaxed, Ordering::Relaxed)
            .is_ok()
        {
            break;
        }
    }
}

pub fn record_compile_error() {
    COMPILE_ERRORS.fetch_add(1, Ordering::Relaxed);
}

pub fn record_rehydrate() {
    REHYDRATE_REQUESTS.fetch_add(1, Ordering::Relaxed);
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MetricsSnapshot {
    pub compile_requests: u64,
    pub compile_errors: u64,
    pub compile_duration_ms_total: u64,
    pub compile_duration_ms_max: u64,
    pub last_compile_duration_ms: u64,
    pub compile_duration_ms_avg: f64,
    pub rehydrate_requests: u64,
    pub latency_histogram: LatencyHistogram,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LatencyHistogram {
    pub le_1ms: u64,
    pub le_5ms: u64,
    pub le_10ms: u64,
    pub le_50ms: u64,
    pub le_200ms: u64,
    pub gt_200ms: u64,
}

pub fn snapshot() -> MetricsSnapshot {
    let requests = COMPILE_REQUESTS.load(Ordering::Relaxed);
    let total = DURATION_MS_TOTAL.load(Ordering::Relaxed);
    let b = buckets();
    MetricsSnapshot {
        compile_requests: requests,
        compile_errors: COMPILE_ERRORS.load(Ordering::Relaxed),
        compile_duration_ms_total: total,
        compile_duration_ms_max: DURATION_MS_MAX.load(Ordering::Relaxed),
        last_compile_duration_ms: LAST_DURATION_MS.load(Ordering::Relaxed),
        compile_duration_ms_avg: if requests == 0 {
            0.0
        } else {
            total as f64 / requests as f64
        },
        rehydrate_requests: REHYDRATE_REQUESTS.load(Ordering::Relaxed),
        latency_histogram: LatencyHistogram {
            le_1ms: b[0].load(Ordering::Relaxed),
            le_5ms: b[1].load(Ordering::Relaxed),
            le_10ms: b[2].load(Ordering::Relaxed),
            le_50ms: b[3].load(Ordering::Relaxed),
            le_200ms: b[4].load(Ordering::Relaxed),
            gt_200ms: b[5].load(Ordering::Relaxed),
        },
    }
}

pub fn prometheus_text() -> String {
    let s = snapshot();
    format!(
        "# HELP tokenopt_compile_requests_total Total compile requests\n\
         # TYPE tokenopt_compile_requests_total counter\n\
         tokenopt_compile_requests_total {}\n\
         # HELP tokenopt_compile_errors_total Total compile errors\n\
         # TYPE tokenopt_compile_errors_total counter\n\
         tokenopt_compile_errors_total {}\n\
         # HELP tokenopt_compile_duration_ms Last compile duration\n\
         # TYPE tokenopt_compile_duration_ms gauge\n\
         tokenopt_compile_duration_ms {}\n\
         # HELP tokenopt_compile_duration_ms_max Max compile duration\n\
         # TYPE tokenopt_compile_duration_ms_max gauge\n\
         tokenopt_compile_duration_ms_max {}\n\
         # HELP tokenopt_rehydrate_requests_total Rehydrate requests\n\
         # TYPE tokenopt_rehydrate_requests_total counter\n\
         tokenopt_rehydrate_requests_total {}\n",
        s.compile_requests,
        s.compile_errors,
        s.last_compile_duration_ms,
        s.compile_duration_ms_max,
        s.rehydrate_requests,
    )
}
