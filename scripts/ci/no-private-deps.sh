#!/usr/bin/env bash
# Fail when the public workspace depends on private laplace-cloud crates.

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "${ROOT}"

tmp_metadata="$(mktemp)"
trap 'rm -f "${tmp_metadata}"' EXIT

cargo metadata --format-version=1 --all-features >"${tmp_metadata}"

# This workspace's own manifests are exempt. When the open checkout lives *inside* a
# directory named `laplace-cloud` (the usual local submodule layout), every one of its own
# paths matches the private pattern and the check fires on nothing but itself. CI clones
# open standalone, so this filter changes no CI outcome -- it only removes the local
# false positive. A genuine private manifest sits outside ${ROOT} and is still caught.
private_paths="$(
  grep -oE '"manifest_path":"[^"]*(/laplace-cloud/|/closed/|/private/)[^"]*"' "${tmp_metadata}" |
  grep -vF "\"manifest_path\":\"${ROOT}/" || true
)"

if [[ -n "${private_paths}" ]]; then
  echo "public cargo metadata leaked a private manifest path" >&2
  echo "${private_paths}" >&2
  exit 1
fi

if grep -E '"name":"laplace-(axiom|core|dpor|ki-dpor|kraken|probe|probe-adapter|byoc-audit|api|cli)"' "${tmp_metadata}" >/dev/null; then
  echo "public cargo metadata contains a private Laplace package" >&2
  grep -E '"name":"laplace-(axiom|core|dpor|ki-dpor|kraken|probe|probe-adapter|byoc-audit|api|cli)"' "${tmp_metadata}" >&2
  exit 1
fi

manifest_hits="$(
  find Cargo.toml crates examples vendor .github -type f \
    \( -name 'Cargo.toml' -o -name '*.yml' -o -name '*.yaml' \) \
    -not -path './target/*' \
    -print0 |
  xargs -0 grep -nE '(\.\./laplace-cloud|/laplace-cloud/|/closed/|/private/|open/crates|features = \["verification"\]|path = ".*laplace-cloud|laplace-(axiom|core|dpor|ki-dpor|kraken|probe-adapter|byoc-audit|api|cli)[[:space:]]*=|dep:laplace-(axiom|core|dpor|ki-dpor|kraken|probe-adapter|byoc-audit|api|cli)|dep:laplace-probe([][",[:space:]]|$))' || true
)"

if [[ -n "${manifest_hits}" ]]; then
  echo "public manifests/workflows contain private dependency references:" >&2
  echo "${manifest_hits}" >&2
  exit 1
fi

echo "public boundary check passed"
