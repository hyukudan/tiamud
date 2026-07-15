#!/usr/bin/env bash

set -euo pipefail
export LC_ALL=C

if [[ $# -ne 5 ]]; then
  echo "usage: $0 <version> <cargo-target-dir> <web-dist-dir> <output-dir> <source-date-epoch>" >&2
  exit 2
fi

version=$1
cargo_target_dir=$2
web_dist_dir=$3
output_dir=$4
source_date_epoch=$5
project_root=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)

if [[ ! $version =~ ^[0-9A-Za-z][0-9A-Za-z._-]*$ ]]; then
  echo "release version contains unsafe characters: $version" >&2
  exit 2
fi
if [[ ! $source_date_epoch =~ ^(0|[1-9][0-9]*)$ ]]; then
  echo "SOURCE_DATE_EPOCH must be an unsigned integer" >&2
  exit 2
fi
if (( ${#source_date_epoch} > 12 )) || {
  (( ${#source_date_epoch} == 12 )) && [[ $source_date_epoch > 253402300799 ]]
}; then
  echo "SOURCE_DATE_EPOCH is outside the supported Unix timestamp range" >&2
  exit 2
fi
if [[ ! -d $web_dist_dir || ! -f $web_dist_dir/index.html ]]; then
  echo "web distribution is missing index.html: $web_dist_dir" >&2
  exit 2
fi
if find "$web_dist_dir" -type l -print -quit | grep -q .; then
  echo "web distribution must not contain symbolic links: $web_dist_dir" >&2
  exit 2
fi
if find "$web_dist_dir" ! -type d ! -type f -print -quit | grep -q .; then
  echo "web distribution contains an unsupported file type: $web_dist_dir" >&2
  exit 2
fi

readonly release_target=x86_64-unknown-linux-gnu
readonly binary_bundle="tiamud-${version}-${release_target}"
readonly web_bundle="tiamud-web-${version}"
readonly -a binaries=(
  envy-import
  tiamud-admin
  tiamud-assets
  tiamud-migrate-postgres
  tiamud-server
  tiamud-world
)
readonly -a license_files=(
  LICENSE-APACHE
  LICENSE-MIT
  LICENSE-CONTENT.md
  THIRD_PARTY_NOTICES.md
)

node "$project_root/scripts/generate-third-party-notices.mjs" --check

temporary_dir=$(mktemp -d)
trap 'rm -rf "$temporary_dir"' EXIT
umask 022

mkdir -p "$temporary_dir/$binary_bundle/bin" "$temporary_dir/$web_bundle" "$output_dir"
printf '%s\n' "$version" >"$temporary_dir/$binary_bundle/VERSION"

for license_file in "${license_files[@]}"; do
  source_path="$project_root/$license_file"
  if [[ ! -f $source_path ]]; then
    echo "release license file is missing: $source_path" >&2
    exit 1
  fi
  install -m 0644 "$source_path" "$temporary_dir/$binary_bundle/$license_file"
  install -m 0644 "$source_path" "$temporary_dir/$web_bundle/$license_file"
done

for binary in "${binaries[@]}"; do
  source_path="$cargo_target_dir/release/$binary"
  if [[ ! -f $source_path || ! -x $source_path ]]; then
    echo "release binary is missing or not executable: $source_path" >&2
    exit 1
  fi
  install -m 0755 "$source_path" "$temporary_dir/$binary_bundle/bin/$binary"
done

if ! "$temporary_dir/$binary_bundle/bin/tiamud-world" ai-draft --help >/dev/null 2>&1; then
  echo "release tiamud-world is missing the authoring-only editorial-ai feature" >&2
  exit 1
fi

cp -a "$web_dist_dir/." "$temporary_dir/$web_bundle/"
printf '%s\n' "$version" >"$temporary_dir/$web_bundle/VERSION"
find "$temporary_dir/$web_bundle" -type d -exec chmod 0755 {} +
find "$temporary_dir/$web_bundle" -type f -exec chmod 0644 {} +

create_archive() {
  local bundle=$1
  local destination="$output_dir/${bundle}.tar.gz"
  local temporary_archive="$temporary_dir/${bundle}.tar.gz"

  tar \
    --sort=name \
    --format=gnu \
    --mtime="@${source_date_epoch}" \
    --owner=0 \
    --group=0 \
    --numeric-owner \
    -C "$temporary_dir" \
    -cf - \
    "$bundle" | gzip -n -9 >"$temporary_archive"
  install -m 0644 "$temporary_archive" "$destination"
}

create_archive "$binary_bundle"
create_archive "$web_bundle"
