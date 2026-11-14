#![allow(missing_docs, clippy::unwrap_used, clippy::expect_used)]
//! Microbenchmarks measuring Ironic module graph compilation, metrics recording, and cryptographic hashing.

use std::time::Instant;

use agents_server::{
    app::InfraModule,
    infra::{DomainMetrics, apply_observability_plugins},
};
use ironic::{ModuleDefinition, metrics::MetricsRegistry};
use sha2::{Digest, Sha256};

fn benchmark<F: FnMut()>(name: &str, mut f: F, iterations: u32) {
    // Warmup
    for _ in 0..(iterations / 10).max(5) {
        f();
    }

    let started = Instant::now();
    for _ in 0..iterations {
        f();
    }
    let elapsed = started.elapsed();
    let per_op_ns = elapsed.as_nanos() / u128::from(iterations);
    println!(
        "bench: {:<40} {:>10} ops  {:>10} ns/op",
        name, iterations, per_op_ns
    );
}

fn main() {
    println!("\n=== Running Agents Ironic Pipeline Microbenchmarks ===\n");

    // 1. Metric counter increment and registry scraping
    let counter = MetricsRegistry.counter("bench_counter_total", "Benchmark test counter");
    benchmark(
        "metrics::counter_increment",
        || {
            counter.inc();
        },
        50_000,
    );

    benchmark(
        "metrics::registry_scrape",
        || {
            let _ = ironic::metrics::scrape();
        },
        5_000,
    );

    // 2. Domain metrics histogram recording
    let domain = DomainMetrics::global();
    benchmark(
        "metrics::domain_histogram_record",
        || {
            domain.request_latency_ms.record(0.015);
        },
        50_000,
    );

    // 3. SHA-256 audit entry hash chaining
    let payload = b"{\"event\":\"session.create\",\"actor\":\"admin\",\"timestamp\":1700000000}";
    let prev_hash = "0000000000000000000000000000000000000000000000000000000000000000";
    benchmark(
        "crypto::sha256_audit_chain",
        || {
            let mut hasher = Sha256::new();
            hasher.update(prev_hash.as_bytes());
            hasher.update(payload);
            let _ = hex::encode(hasher.finalize());
        },
        50_000,
    );

    // 4. Observability plugin application via PluginRegistry
    benchmark(
        "plugin::observability_apply",
        || {
            let builder = ModuleDefinition::builder::<InfraModule>();
            let _ = apply_observability_plugins(builder);
        },
        10_000,
    );

    println!("\n=== Agents Ironic Pipeline Microbenchmarks Finished ===\n");
}
