#!/bin/bash -eu
# Compile every libFuzzer target and copy the binaries to $OUT, as
# ClusterFuzzLite/OSS-Fuzz expect. base-builder-rust supplies cargo-fuzz and the
# RUSTFLAGS for the active sanitizer.
cd "$SRC/yamlrocks"

# Compile the PyO3 layer against the standalone CPython 3.12 installed in the
# Dockerfile; the base image's default Python is too old (PyList subclassing
# needs Py_3_12). Only the interpreter is used at build time.
PYO3_PYTHON="$(uv python find 3.12)"
export PYO3_PYTHON

cargo fuzz build -O

# The release directory lives under a target-triple subdirectory; there is
# exactly one, so a glob resolves it without hardcoding the triple.
for target in fuzz/fuzz_targets/*.rs; do
    name="$(basename "${target%.*}")"
    cp fuzz/target/*/release/"${name}" "$OUT/"
done
