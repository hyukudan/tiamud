#!/usr/bin/env bash

set -euo pipefail

project_root=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)
readonly project_root
# Reviewed complete project license texts. The MIT digest includes the Tiamud
# contributor copyright line; Apache and CC match their official plain text.
readonly mit_sha256=f95d257d1cde3ccb0b5df2d00448fef3df14884ee55db5ab775cf221fe9e8458
readonly apache_2_sha256=cfc7749b96f63bd31c3c42b5c471bf756814053e847c10f3eb003417bc523d30
# Official plain-text legal code:
# https://creativecommons.org/licenses/by/4.0/legalcode.txt
readonly cc_by_4_sha256=9ba9550ad48438d0836ddab3da480b3b69ffa0aac7b7878b5a0039e7ab429411
readonly -a required_files=(
  LICENSE
  LICENSE-APACHE
  LICENSE-MIT
  LICENSE-CONTENT.md
  LICENSE-CC-BY-4.0
)

for file in "${required_files[@]}"; do
  if [[ ! -f "$project_root/$file" ]]; then
    echo "required project license file is missing: $file" >&2
    exit 1
  fi
done

verify_digest() {
  local file=$1
  local expected=$2
  local description=$3
  local actual
  actual=$(sha256sum "$project_root/$file")
  actual=${actual%% *}
  if [[ $actual != "$expected" ]]; then
    echo "$file differs from $description" >&2
    echo "expected: $expected" >&2
    echo "actual:   $actual" >&2
    exit 1
  fi
}

verify_digest LICENSE-MIT "$mit_sha256" "the reviewed Tiamud MIT license text"
verify_digest LICENSE-APACHE "$apache_2_sha256" "the official Apache-2.0 plain-text legal code"
verify_digest LICENSE-CC-BY-4.0 "$cc_by_4_sha256" "the official CC BY 4.0 plain-text legal code"

echo "verified project license files and reviewed complete license texts"
