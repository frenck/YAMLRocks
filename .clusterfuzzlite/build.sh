#!/bin/bash -eu
# Compile every libFuzzer target and copy the binaries to $OUT, as
# ClusterFuzzLite/OSS-Fuzz expect. base-builder-rust supplies cargo-fuzz and the
# RUSTFLAGS for the active sanitizer.
cd "$SRC/yamlrocks"

cargo fuzz build -O

# The release directory lives under a target-triple subdirectory; there is
# exactly one, so a glob resolves it without hardcoding the triple.
for target in fuzz/fuzz_targets/*.rs; do
    name="$(basename "${target%.*}")"
    cp fuzz/target/*/release/"${name}" "$OUT/"
done
