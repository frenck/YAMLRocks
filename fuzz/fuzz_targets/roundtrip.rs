#![no_main]

use libfuzzer_sys::fuzz_target;

// Round-trip arbitrary text through compose → emit → re-compose. This exercises
// the round-trip emitter (the code behind `to_yaml`/`save`), which the `parse`
// and `decode` targets do not reach. The contract is the same as the other
// targets: never panic, never hang, never trigger undefined behavior, across
// both the emit and the re-parse of its output. See `_yamlrocks::fuzz::roundtrip`
// for why this stops short of asserting strict re-parse fidelity.
fuzz_target!(|data: &[u8]| {
    // The composer operates on text; feed only valid UTF-8 (invalid UTF-8 is
    // rejected before composing and is not interesting to fuzz here).
    if let Ok(text) = std::str::from_utf8(data) {
        _yamlrocks::fuzz::roundtrip(text);
    }
});
