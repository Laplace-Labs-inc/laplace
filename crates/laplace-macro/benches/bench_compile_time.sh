#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
set -euo pipefail

SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
CRATE_DIR="$(cd -- "${SCRIPT_DIR}/.." && pwd)"
WORKSPACE_DIR="$(cd -- "${CRATE_DIR}/../.." && pwd)"
FIXTURE_DIR="${SCRIPT_DIR}/fixtures/compile_time"

MODE="${1:-${LAPLACE_BENCH_MODE:-verify}}"
RUNS="${RUNS:-30}"
WARMUP_RUNS="${WARMUP_RUNS:-10}"
BENCH_TARGET_DIR="${BENCH_TARGET_DIR:-${WORKSPACE_DIR}/target/laplace-macro-compile-time}"

export LAPLACE_BENCH_SEED="${LAPLACE_BENCH_SEED:-42}"
export CARGO_INCREMENTAL="${CARGO_INCREMENTAL:-0}"
export CARGO_TERM_COLOR="${CARGO_TERM_COLOR:-never}"

print_environment() {
  printf 'bench_environment crate=laplace-macro mode=%s seed=%s cargo_incremental=%s target_dir=%s\n' \
    "${MODE}" "${LAPLACE_BENCH_SEED}" "${CARGO_INCREMENTAL}" "${BENCH_TARGET_DIR}"
}

verify_only() {
  print_environment
  cargo check \
    --manifest-path "${FIXTURE_DIR}/Cargo.toml" \
    --target-dir "${BENCH_TARGET_DIR}" \
    --locked \
    --quiet
}

clean_fixture() {
  cargo clean \
    --manifest-path "${FIXTURE_DIR}/Cargo.toml" \
    --target-dir "${BENCH_TARGET_DIR}" \
    -p macro-compile-time-fixture \
    --quiet
}

run_once() {
  local start_ns end_ns

  clean_fixture
  start_ns="$(date +%s%N)"
  cargo check \
    --manifest-path "${FIXTURE_DIR}/Cargo.toml" \
    --target-dir "${BENCH_TARGET_DIR}" \
    --locked \
    --quiet
  end_ns="$(date +%s%N)"

  printf '%s\n' "$((end_ns - start_ns))"
}

measure() {
  print_environment

  for _ in $(seq 1 "${WARMUP_RUNS}"); do
    run_once >/dev/null
  done

  for run_id in $(seq 1 "${RUNS}"); do
    elapsed_ns="$(run_once)"
    printf 'macro_proc_macro_expansion_compile_time_ns run=%s seed=%s value=%s fixture=compile_time\n' \
      "${run_id}" "${LAPLACE_BENCH_SEED}" "${elapsed_ns}"
  done
}

case "${MODE}" in
  verify|--verify|--verify-only)
    verify_only
    ;;
  measure|--measure)
    measure
    ;;
  *)
    printf 'usage: %s [--verify-only|--measure]\n' "$0" >&2
    exit 2
    ;;
esac
