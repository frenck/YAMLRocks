#![no_main]

use libfuzzer_sys::fuzz_target;

// Drive arbitrary text through the full YAML parse pipeline (scanner → parser →
// composer). The contract under fuzzing: never panic, never hang, never trigger
// undefined behavior. A malformed document must surface as a clean parse error,
// not a crash. Coverage-guided mutation explores the scanner's many states far
// more thoroughly than the random property tests in tests/robustness/.
fuzz_target!(|data: &[u8]| {
    // The parser operates on text; feed only valid UTF-8 (invalid UTF-8 is
    // rejected before parsing and is not interesting to fuzz here).
    if let Ok(text) = std::str::from_utf8(data) {
        _yamlrocks::fuzz::parse(text);
    }
});
