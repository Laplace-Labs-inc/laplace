// SPDX-License-Identifier: Apache-2.0
//! Benchmark: serde_json serialization vs Laplace-LZ 3-stage compression pipeline.
//!
//! Compares:
//! - JSON-only path  : `serde_json::to_vec(&data)` → raw JSON wire bytes
//! - Laplace-LZ path : JSON → SemanticEncoder (L1) → LZ4 (L3) → LaplaceContext header
//!
//! Run with: `cargo bench -p laplace-probe`

use criterion::{black_box, criterion_group, criterion_main, Criterion};
use serde::{Deserialize, Serialize};

use laplace_probe::{
    domain::{
        context::{CONTEXT_BYTES, CTX_FLAG},
        wire::lz4_compress,
    },
    LaplaceContext, SemanticEncoder, StaticDictionary, FLAG_LAYER1, FLAG_LAYER3,
    LZ4_COMPRESSION_THRESHOLD,
};

// ── Dummy data structures ─────────────────────────────────────────────────────

#[derive(Serialize, Deserialize, Clone)]
struct Tag {
    id: u32,
    name: String,
    color: String,
}

#[derive(Serialize, Deserialize, Clone)]
struct Attribute {
    key: String,
    value: String,
    metadata: Vec<String>,
}

#[derive(Serialize, Deserialize, Clone)]
struct OrderItem {
    item_id: u64,
    product_id: String,
    name: String,
    description: String,
    quantity: u32,
    unit_price: f64,
    discount_pct: f64,
    tags: Vec<Tag>,
    attributes: Vec<Attribute>,
}

#[derive(Serialize, Deserialize, Clone)]
struct Address {
    line1: String,
    line2: String,
    city: String,
    state: String,
    country: String,
    postal_code: String,
    latitude: f64,
    longitude: f64,
}

#[derive(Serialize, Deserialize, Clone)]
struct Order {
    order_id: String,
    created_at: u64,
    status: String,
    currency: String,
    total_amount: f64,
    shipping_address: Address,
    billing_address: Address,
    items: Vec<OrderItem>,
    notes: String,
    tracking_numbers: Vec<String>,
}

#[derive(Serialize, Deserialize, Clone)]
struct Pagination {
    page: u32,
    per_page: u32,
    total_count: u64,
    has_next: bool,
    has_prev: bool,
    cursor: String,
}

#[derive(Serialize, Deserialize, Clone)]
struct ApiResponse {
    request_id: String,
    api_version: String,
    status: String,
    timestamp_ns: u64,
    user_id: u64,
    session_token: String,
    tenant_id: String,
    region: String,
    orders: Vec<Order>,
    pagination: Pagination,
    diagnostics: Vec<String>,
}

// ── Dummy data generator ──────────────────────────────────────────────────────

fn make_tag(i: u32) -> Tag {
    Tag {
        id: i,
        name: format!("tag-category-{i}-label-extended"),
        color: format!("#{:06x}", i * 12345 % 0xFFFFFF),
    }
}

fn make_attribute(i: usize) -> Attribute {
    Attribute {
        key: format!("attribute_key_{i}_extended_name"),
        value: format!("attribute_value_{i}_with_some_longer_content_to_bulk_up_payload"),
        metadata: (0..3).map(|j| format!("meta-{i}-{j}")).collect(),
    }
}

fn make_order_item(i: usize) -> OrderItem {
    OrderItem {
        item_id: i as u64 * 10_000,
        product_id: format!("PROD-{:08x}", i * 99991),
        name: format!("Product Item #{i} — Extended Description for Benchmark Padding"),
        description: concat!(
            "A representative product with enough textual content to simulate real-world ",
            "API payloads. This description intentionally includes repeated phrases and ",
            "metadata to generate a payload that exercises the LZ4 compression algorithm ",
            "effectively. Fields like this are common in e-commerce and logistics APIs."
        )
        .to_string(),
        quantity: (i % 10 + 1) as u32,
        unit_price: 9.99 + i as f64 * 1.23,
        discount_pct: (i % 20) as f64 * 0.5,
        tags: (0..5).map(|t| make_tag(t as u32)).collect(),
        attributes: (0..4).map(make_attribute).collect(),
    }
}

fn make_address(prefix: &str, i: usize) -> Address {
    const CITIES: [&str; 5] = ["New York", "Los Angeles", "Chicago", "Houston", "Phoenix"];
    const STATES: [&str; 5] = ["NY", "CA", "IL", "TX", "AZ"];
    Address {
        line1: format!("{i} {prefix} Main Street, Suite {}", i * 3 + 100),
        line2: format!("Building {}, Floor {}", (i % 5) + 1, (i % 10) + 1),
        city: CITIES[i % 5].to_string(),
        state: STATES[i % 5].to_string(),
        country: "US".to_string(),
        postal_code: format!("{:05}", 10000 + i * 137),
        latitude: 37.7749 + i as f64 * 0.001,
        longitude: -122.4194 + i as f64 * 0.001,
    }
}

fn make_order(i: usize) -> Order {
    const STATUSES: [&str; 5] = ["pending", "processing", "shipped", "delivered", "cancelled"];
    Order {
        order_id: format!(
            "ORD-{:016x}",
            (i as u64)
                .wrapping_mul(0xDEAD_BEEF)
                .wrapping_add(0xCAFE_1234)
        ),
        created_at: 1_700_000_000 + i as u64 * 3600,
        status: STATUSES[i % 5].to_string(),
        currency: "USD".to_string(),
        total_amount: 100.0 + i as f64 * 17.83,
        shipping_address: make_address("Shipping", i),
        billing_address: make_address("Billing", i),
        items: (0..8).map(make_order_item).collect(),
        notes: format!(
            "Order note {i}: customer comment with reference REF-{:010}. \
             This note is intentionally verbose to represent real-world data patterns.",
            i * 7919
        ),
        tracking_numbers: (0..3)
            .map(|t| format!("TRACK-{i:04}-{t:04}-{:08}", i * (t + 1) * 1234 + 5678))
            .collect(),
    }
}

/// Build an `ApiResponse` with `num_orders` orders (~50 KB for num_orders=20).
fn make_dummy_response(num_orders: usize) -> ApiResponse {
    ApiResponse {
        request_id: "req-550e8400-e29b-41d4-a716-446655440000".to_string(),
        api_version: "v2.1.0".to_string(),
        status: "success".to_string(),
        timestamp_ns: 1_700_000_000_000_000_000,
        user_id: 987_654_321,
        session_token: concat!(
            "eyJhbGciOiJSUzI1NiIsInR5cCI6IkpXVCJ9.",
            "eyJzdWIiOiJ1c2VyLTk4NzY1NDMyMSIsImlhdCI6MTcwMDAwMDAwMCwiZXhwIjoxNzAwMDg2NDAwfQ.",
            "mock_signature_for_benchmark_padding_purposes_only"
        )
        .to_string(),
        tenant_id: "tenant-acme-corp-production".to_string(),
        region: "us-west-2".to_string(),
        orders: (0..num_orders).map(make_order).collect(),
        pagination: Pagination {
            page: 1,
            per_page: num_orders as u32,
            total_count: num_orders as u64 * 100,
            has_next: true,
            has_prev: false,
            cursor: "eyJwYWdlIjoyLCJwZXJfcGFnZSI6MjB9".to_string(),
        },
        diagnostics: (0..10)
            .map(|i| {
                format!(
                    "diag[{i}]: latency={:.3}ms db_queries={} cache_hits={}",
                    i as f64 * 0.123,
                    i * 3,
                    i * 7
                )
            })
            .collect(),
    }
}

// ── Laplace pipeline helpers ──────────────────────────────────────────────────

fn make_static_dict() -> StaticDictionary {
    let mut d = StaticDictionary::new();
    for ep in [
        "POST /api/orders",
        "GET /api/orders",
        "GET /api/users",
        "PUT /api/orders",
        "DELETE /api/orders",
        "GET /api/products",
        "POST /api/users",
    ] {
        d.insert(ep.to_string()).unwrap();
    }
    d
}

fn make_context() -> LaplaceContext {
    LaplaceContext {
        trace_id: 0xDEAD_BEEF_CAFE_1234_ABCD_EF01_2345_6789,
        tenant_id: 1001,
        virtual_clock_ns: 1_700_000_000_000_000_000,
        lamport_tick: 42,
        priority: 128,
    }
}

/// Full Laplace-LZ wire frame:
/// `[4B len BE][1B flags][41B LaplaceContext LE][N bytes: LZ4(SemanticEncode(payload))]`
///
/// Applies Layer 1 (static dict VarInt) and Layer 3 (LZ4) if payload > threshold.
fn encode_laplace(dict: &StaticDictionary, ctx: &LaplaceContext, json_bytes: &[u8]) -> Vec<u8> {
    // Layer 1: prepend VarInt route_id to the JSON payload
    let l1_frame = SemanticEncoder::encode(dict, "POST /api/orders", json_bytes).unwrap();

    // Layer 3: LZ4-compress when above the 4 KiB threshold
    let (final_payload, layer3_active) = if l1_frame.len() > LZ4_COMPRESSION_THRESHOLD {
        (lz4_compress(&l1_frame).unwrap(), true)
    } else {
        (l1_frame, false)
    };

    // Assemble frame header flags
    let mut frame_flags: u8 = CTX_FLAG | FLAG_LAYER1;
    if layer3_active {
        frame_flags |= FLAG_LAYER3;
    }

    let ctx_bytes = ctx.to_bytes();
    let total_frame_len = (1 + CONTEXT_BYTES + final_payload.len()) as u32;

    let mut frame = Vec::with_capacity(4 + 1 + CONTEXT_BYTES + final_payload.len());
    frame.extend_from_slice(&total_frame_len.to_be_bytes());
    frame.push(frame_flags);
    frame.extend_from_slice(&ctx_bytes);
    frame.extend_from_slice(&final_payload);
    frame
}

// ── Criterion benchmark ───────────────────────────────────────────────────────

fn bench_serialization_speed(c: &mut Criterion) {
    // ~50 KB response (20 orders × 8 items each)
    let data = make_dummy_response(20);
    let dict = make_static_dict();
    let ctx = make_context();

    // Pre-compute once to print size comparison before the loops start
    let json_bytes = serde_json::to_vec(&data).unwrap();
    let laplace_frame = encode_laplace(&dict, &ctx, &json_bytes);

    println!("\n╔══════════════════════════════════════════╗");
    println!("║      Payload Size Comparison (눈바디)     ║");
    println!("╠══════════════════════════════════════════╣");
    println!(
        "║  Raw JSON size     : {:>8} bytes       ║",
        json_bytes.len()
    );
    println!(
        "║  Laplace frame size: {:>8} bytes       ║",
        laplace_frame.len()
    );
    println!(
        "║  Space saved       : {:>8} bytes ({:.1}%) ║",
        json_bytes.len() as i64 - laplace_frame.len() as i64,
        (1.0 - laplace_frame.len() as f64 / json_bytes.len() as f64) * 100.0
    );
    println!("╚══════════════════════════════════════════╝\n");

    let mut group = c.benchmark_group("Serialization_Speed");

    // 비교군: 순수 JSON 직렬화 속도 (약 690 µs 예상)
    group.bench_function("serde_json::to_vec", |b| {
        b.iter(|| serde_json::to_vec(black_box(&data)).unwrap())
    });

    // 타겟: 순수 Laplace 파이프라인(압축+컨텍스트) 속도
    group.bench_function("laplace_lz_full_pipeline_only", |b| {
        b.iter(|| {
            // 미리 만들어둔 바이트 배열을 바로 넘겨서 순수 압축 시간만 측정!
            encode_laplace(black_box(&dict), black_box(&ctx), black_box(&json_bytes))
        })
    });

    group.finish();
}

criterion_group!(benches, bench_serialization_speed);
criterion_main!(benches);
