#!/bin/sh
# Builds every source in `sources/` and writes a repository to a folder,
# `dist` by default: one component per source and the `index.json` listing
# them. Serve the folder over HTTPS and add its URL in Chek.
set -eu
cd "$(dirname "$0")"
out="${1:-dist}"
name="${2:-Chek}"

cargo build --release --target wasm32-wasip2
mkdir -p "$out"
rm -f "$out"/*.wasm "$out"/index.json
for dir in sources/*/; do
    crate=$(basename "$dir")
    version=$(sed -n 's/^version = "\(.*\)"/\1/p' "$dir/Cargo.toml" | head -n 1)
    # The version in the name keeps a cached older component from being
    # mistaken for a new one.
    cp "target/wasm32-wasip2/release/$(echo "$crate" | tr - _).wasm" "$out/$crate-$version.wasm"
done
cargo run --quiet --manifest-path ../Cargo.toml -p chek-sources --example extension-index -- "$out" "$name"
