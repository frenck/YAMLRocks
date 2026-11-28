#![no_main]

use libfuzzer_sys::fuzz_target;

// Differential target for the round-trip emitter. For text that loads, it
// re-emits the round-trip tree through the emitter (never the cached source),
// reloads the output, and asserts the data is unchanged; then marks one scalar
// value modified and checks again. Where `roundtrip` only proves the emitter
// does not crash, this catches an edited document's `to_yaml` writing YAML that
// reads back as different data. See `_yamlrocks::fuzz::roundtrip_differential`.
fuzz_target!(|data: &[u8]| {
    if let Ok(text) = std::str::from_utf8(data) {
        _yamlrocks::fuzz::roundtrip_differential(text);
    }
});
