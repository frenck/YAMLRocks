#![no_main]

use libfuzzer_sys::fuzz_target;

// Drive arbitrary text through the fast decode path (scanner → parser →
// resolver → `Value` tree → fast emitter), under both the YAML 1.2 and 1.1
// schemas. This is the code that plain `loads`/`dumps` run, distinct from the
// round-trip composer the `parse` target covers. The contract: never panic,
// never hang, never trigger undefined behavior; a malformed document must
// surface as a clean decode error, not a crash.
fuzz_target!(|data: &[u8]| {
    // The decoder operates on text; feed only valid UTF-8 (invalid UTF-8 is
    // rejected before decoding and is not interesting to fuzz here).
    if let Ok(text) = std::str::from_utf8(data) {
        _yamlrocks::fuzz::decode(text);
    }
});
