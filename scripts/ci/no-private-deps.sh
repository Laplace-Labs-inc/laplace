#!/usr/bin/env bash
# Fail when the public workspace depends on private laplace-cloud crates.

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "${ROOT}"

tmp_metadata="$(mktemp)"
trap 'rm -f "${tmp_metadata}"' EXIT

cargo metadata --format-version=1 --all-features >"${tmp_metadata}"

if grep -E '"manifest_path":".*(/laplace-cloud/|/closed/|/private/)' "${tmp_metadata}" >/dev/null; then
  echo "public cargo metadata leaked a private manifest path" >&2
  grep -E '"manifest_path":".*(/laplace-cloud/|/closed/|/private/)' "${tmp_metadata}" >&2
  exit 1
fi

if grep -E '"name":"laplace-(axiom|core|ki-dpor|kraken|probe|probe-adapter|byoc-audit|api|cli)"' "${tmp_metadata}" >/dev/null; then
  echo "public cargo metadata contains a private Laplace package" >&2
  grep -E '"name":"laplace-(axiom|core|ki-dpor|kraken|probe|probe-adapter|byoc-audit|api|cli)"' "${tmp_metadata}" >&2
  exit 1
fi

manifest_hits="$(
  find Cargo.toml crates examples vendor .github -type f \
    \( -name 'Cargo.toml' -o -name '*.yml' -o -name '*.yaml' \) \
    -not -path './target/*' \
    -print0 |
  xargs -0 grep -nE '(\.\./laplace-cloud|/laplace-cloud/|/closed/|/private/|path = ".*laplace-cloud|dep:laplace-(axiom|core|ki-dpor|kraken|probe-adapter|byoc-audit|api|cli)|dep:laplace-probe([][",[:space:]]|$))' || true
)"

if [[ -n "${manifest_hits}" ]]; then
  echo "public manifests/workflows contain private dependency references:" >&2
  echo "${manifest_hits}" >&2
  exit 1
fi

echo "public boundary check passed"
