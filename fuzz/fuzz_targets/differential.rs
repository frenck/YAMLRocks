#![no_main]

use libfuzzer_sys::fuzz_target;

// Differential target: decode arbitrary text to a `Value` tree, re-emit it with
// the fast `dumps` path, decode that output again, and assert the two trees
// carry identical data. Where `parse`/`decode`/`roundtrip` check the contract
// "never crash", this one checks "never silently corrupt": `dumps` must never
// produce YAML that `loads` reads back as different values (a mis-quoted string
// that re-resolves to a bool, a float that loses precision, a key that shifts).
// That failure shows up as wrong data, not a panic, so only a comparison can
// surface it. See `_yamlrocks::fuzz::differential` for why it is YAML-1.2-only
// and why re-decode failures are skipped rather than asserted.
fuzz_target!(|data: &[u8]| {
    // The decoder operates on text; feed only valid UTF-8 (invalid UTF-8 is
    // rejected before decoding and is not interesting to fuzz here).
    if let Ok(text) = std::str::from_utf8(data) {
        _yamlrocks::fuzz::differential(text);
    }
});
