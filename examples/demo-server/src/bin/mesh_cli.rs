// SPDX-License-Identifier: Apache-2.0
//! Laplace Mesh CLI — Semantic Mesh Bidirectional Test Client (QUIC)
//!
//! Creates a test dictionary, encodes a target endpoint + payload into a single
//! compressed binary frame via [`laplace_probe::domain::wire::SemanticEncoder`], sends the
//! frame over QUIC to a running [`laplace-probe`], and inflates the compressed
//! acknowledgement response back to a human-readable representation.
//!
//! # Usage
//! ```text
//! # Terminal 1: start the agent
//! cargo run -p laplace-probe
//!
//! # Terminal 2: fire the compressed frame over QUIC
//! cargo run -p laplace-mesh-cli
//! ```
//!
//! # Phase 5.8: QUIC Native Transport (TCP → KNUL)
//! ```text
//! CLI ──[QUIC stream]──► Agent: decode → encode response ──[QUIC stream]──► CLI: decode → log
//! ```
//!
//! # Wire format (request & response share the same layout)
//! ```text
//! ┌─────────┬──────────────────────────────┐
//! │  1-byte │  N bytes                     │
//! │   ID    │  JSON payload                │
//! └─────────┴──────────────────────────────┘
//! ```
//!
//! # Lifetime design
//! `endpoint` → `conn` → `stream` are all kept in `main` scope to prevent
//! early Drop that would abort the underlying QUIC connection mid-flight.

use laplace_interfaces::domain::transport::KnulEndpoint;
use laplace_probe::domain::wire::StaticDictionary;
use laplace_probe::infrastructure::transport::quinn_impl::QuinnEndpoint;

#[cfg(feature = "scribe_docs")]
use laplace_macro::laplace_meta;

const AGENT_ADDR: &str = "127.0.0.1:8080";
const TARGET_ENDPOINT: &str = "GET /api/pets";
const PAYLOAD: &[u8] = br#"{"page": 1}"#;
/// Maximum response frame size buffered from the agent (4 KiB is ample).
const MAX_RESPONSE_BYTES: usize = 4_096;

#[tokio::main]
#[cfg_attr(
    feature = "scribe_docs",
    laplace_meta(
        layer = "60_Console_MeshCLI",
        link = "LEP-0017-laplace-probe-cli_protocol_hacking_and_teardown"
    )
)]
async fn main() {
    // ── 1. Build test StaticDictionary ────────────────────────────────────────
    println!("[mesh-cli] Building test StaticDictionary");
    let mut dict = StaticDictionary::new();

    // Add test endpoints matching laplace-probe's dictionary
    let test_endpoints = vec![
        "GET /api/pets",
        "POST /api/pets",
        "GET /api/pets/{id}",
        "PUT /api/pets/{id}",
        "DELETE /api/pets/{id}",
        "GET /api/store/inventory",
        "POST /api/store/order",
        "GET /api/user/{username}",
        "POST /api/user",
    ];

    for ep in &test_endpoints {
        if let Err(e) = dict.insert(ep.to_string()) {
            eprintln!(
                "[mesh-cli] ERROR: Failed to insert endpoint '{}': {}",
                ep, e
            );
            std::process::exit(1);
        }
    }

    println!(
        "[mesh-cli] StaticDictionary ready — {} entries loaded",
        test_endpoints.len()
    );

    // ── 2. Resolve target endpoint ID ────────────────────────────────────────
    match dict.get_id(TARGET_ENDPOINT) {
        Some(id) => println!("[mesh-cli] Endpoint '{TARGET_ENDPOINT}' → ID 0x{id:02X}"),
        None => {
            eprintln!("[mesh-cli] ERROR: '{TARGET_ENDPOINT}' not found in dictionary.");
            std::process::exit(1);
        }
    }

    // ── 3. Encode request → [ID] + [Payload] ─────────────────────────────────
    let frame =
        match laplace_probe::domain::wire::SemanticEncoder::encode(&dict, TARGET_ENDPOINT, PAYLOAD)
        {
            Ok(f) => f,
            Err(e) => {
                eprintln!("[mesh-cli] ERROR: Encode failed: {e}");
                std::process::exit(1);
            }
        };

    println!(
        "[mesh-cli] Sending compressed frame: {:02X?}",
        frame.as_slice()
    );
    println!("[mesh-cli]   ├─ ID byte  : 0x{:02X}", frame[0]);
    println!(
        "[mesh-cli]   └─ Payload  : {} bytes — {}",
        frame.len() - 1,
        std::str::from_utf8(&frame[1..]).unwrap_or("<binary>")
    );

    // ── 4. Connect to laplace-probe over QUIC ─────────────────────────────────
    // IMPORTANT: endpoint, conn, stream are all kept in main scope to prevent
    // early Drop that would abort the underlying QUIC connection mid-flight.
    println!("[mesh-cli] Connecting to agent at {AGENT_ADDR} via QUIC ...");
    let mut endpoint = QuinnEndpoint::new();

    let mut conn = match endpoint.connect_client(AGENT_ADDR).await {
        Ok(c) => {
            println!("[mesh-cli] QUIC connection established");
            c
        }
        Err(e) => {
            eprintln!("[mesh-cli] ERROR: Could not connect to {AGENT_ADDR} via QUIC: {e:?}");
            eprintln!("[mesh-cli] Is laplace-probe running? (cargo run -p laplace-probe)");
            std::process::exit(1);
        }
    };

    let mut stream = match conn.open_stream().await {
        Ok(s) => s,
        Err(e) => {
            eprintln!("[mesh-cli] ERROR: Could not open stream: {e:?}");
            std::process::exit(1);
        }
    };

    // ── 5. Write request frame ────────────────────────────────────────────────
    if let Err(e) = stream.write(&frame).await {
        eprintln!("[mesh-cli] ERROR: Write failed: {e:?}");
        std::process::exit(1);
    }

    // Half-close the write side so the agent sees EOF and knows the request
    // is complete. The read side stays open to receive the response frame.
    if let Err(e) = stream.close().await {
        eprintln!("[mesh-cli] ERROR: Could not close write side: {e:?}");
    }

    println!(
        "[mesh-cli] Request sent ({} bytes total). Waiting for response ...",
        frame.len()
    );

    // ── 6. Read response frame from agent ─────────────────────────────────────
    let mut resp_buf = vec![0u8; MAX_RESPONSE_BYTES];
    let n = match stream.read(&mut resp_buf).await {
        Ok(n) => n,
        Err(e) => {
            eprintln!("[mesh-cli] ERROR: Response read failed: {e:?}");
            std::process::exit(1);
        }
    };

    if n == 0 {
        eprintln!("[mesh-cli] ERROR: Agent closed stream without sending a response.");
        std::process::exit(1);
    }

    let resp_frame = &resp_buf[..n];

    // ── 7. Decode response frame ──────────────────────────────────────────────
    match laplace_probe::domain::wire::SemanticDecoder::decode(&dict, resp_frame) {
        Ok((resp_endpoint, resp_payload)) => {
            let status = if resp_endpoint.starts_with("GET ")
                || resp_endpoint.starts_with("POST ")
                || resp_endpoint.starts_with("PUT ")
                || resp_endpoint.starts_with("DELETE ")
                || resp_endpoint.starts_with("PATCH ")
                || resp_endpoint.starts_with("HEAD ")
                || resp_endpoint.starts_with("OPTIONS ")
            {
                "200 OK"
            } else {
                "500 Internal Server Error"
            };

            let payload_str = std::str::from_utf8(resp_payload).unwrap_or("<binary>");

            println!("[mesh-cli] ── Response received ──────────────────────────");
            println!("[mesh-cli] Received Response: [{status}]");
            println!("[mesh-cli]   ├─ Endpoint : {resp_endpoint}");
            println!("[mesh-cli]   ├─ Payload  : {payload_str}");
            println!("[mesh-cli]   └─ Raw      : {:02X?}", resp_frame);
        }
        Err(e) => {
            eprintln!("[mesh-cli] ERROR: Response decode failed: {e}");
            std::process::exit(1);
        }
    }

    // endpoint, conn, stream are dropped HERE — after all I/O is complete.
    drop(stream);
    drop(conn);
    drop(endpoint);
}
