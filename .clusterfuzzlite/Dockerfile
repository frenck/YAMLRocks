# Build environment for ClusterFuzzLite. base-builder-rust provides the Rust
# toolchain, cargo-fuzz, and the sanitizer setup that OSS-Fuzz uses.
FROM gcr.io/oss-fuzz-base/base-builder-rust

COPY . $SRC/yamlrocks
WORKDIR $SRC/yamlrocks
COPY .clusterfuzzlite/build.sh $SRC/build.sh
