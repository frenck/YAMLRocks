# Build environment for ClusterFuzzLite. base-builder-rust provides the Rust
# toolchain, cargo-fuzz, and the sanitizer setup that OSS-Fuzz uses. Pinned by
# digest for a reproducible, supply-chain-safe build; Renovate keeps it current.
FROM gcr.io/oss-fuzz-base/base-builder-rust:latest@sha256:cf0c9d52afd33f354692198cd99402c240d5c467a3a3503908e97a027708075f

# The fuzz targets link the yamlrocks library, which includes the PyO3 extension
# code. Compiling the annotated list/dict pyclasses needs CPython >= 3.12 (PyO3
# gates native PyList/PyDict subclassing on Py_3_12), but the base image ships
# an older Python. Install a standalone CPython 3.12 with uv (independent of the
# base distro) so build.sh can point PyO3 at it. Only the interpreter is needed
# to compile; the fuzz binaries never load Python.
ENV UV_PYTHON_INSTALL_DIR=/opt/uv-python
RUN curl -LsSf https://astral.sh/uv/install.sh | env UV_INSTALL_DIR=/usr/local/bin sh \
    && uv python install 3.12

COPY . $SRC/yamlrocks
WORKDIR $SRC/yamlrocks
COPY .clusterfuzzlite/build.sh $SRC/build.sh
