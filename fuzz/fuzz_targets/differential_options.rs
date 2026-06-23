#![no_main]

use libfuzzer_sys::fuzz_target;

// Differential target over non-default emit options. Like `differential`, it
// decodes arbitrary text and asserts that emitting then re-decoding preserves
// the data, but it emits each document under a matrix of `EmitOptions` (flow
// style, indentation, null styles, quote style, explicit markers, indentless
// sequences, and several line widths) instead of only the defaults. `dumps` must
// never produce YAML that `loads` reads back as different values under *any*
// presentation choice; width-driven line breaking and the indentless layout are
// the prime suspects. See `_yamlrocks::fuzz::differential_options`.
fuzz_target!(|data: &[u8]| {
    if let Ok(text) = std::str::from_utf8(data) {
        _yamlrocks::fuzz::differential_options(text);
    }
});
