#![no_main]

use std::fs;

use libfuzzer_sys::fuzz_target;
use tempfile::TempDir;

// Fuzz the include engine (`!include`, `!include_dir_*`, `!secret`, `!env_var`)
// over a real directory tree, the one part of the library the other targets
// cannot reach because it reads files. The engine is filesystem-coupled, so each
// input is turned into a small tree in a fresh temp directory before being
// resolved. See `_yamlrocks::fuzz::include` for the contract (never panic/hang,
// and no read may escape the base directory).
//
// The input is split on NUL bytes into segments mapped onto fixed file names the
// resolver can reference. The first segment is the root document; the rest
// populate the tree. Fixed names let the fuzzer learn to write directives like
// `!include a.yaml`, a cycle `a.yaml -> b.yaml -> a.yaml`, a diamond fan-out that
// probes the expansion budget, `!secret NAME`, or `!include_dir_list sub` and
// have them actually reach a file.
fuzz_target!(|data: &[u8]| {
    // The root document is parsed as text; a non-UTF-8 input is not interesting
    // here (the whole tree would be unreadable). Individual files may still end
    // up non-UTF-8 and exercise the read-error path.
    let Ok(text) = std::str::from_utf8(data) else {
        return;
    };

    // segment 0 is the root document; the rest map onto these names in order.
    let names = [
        "a.yaml",
        "b.yaml",
        "c.yaml",
        "secrets.yaml",
        "sub/x.yaml",
        "sub/y.yaml",
    ];
    let mut segments = text.split('\u{0}');
    let root = segments.next().unwrap_or("");

    let Ok(dir) = TempDir::new() else {
        return;
    };
    let base = dir.path();
    if fs::create_dir(base.join("sub")).is_err() {
        return;
    }

    for (name, contents) in names.iter().zip(segments) {
        if fs::write(base.join(name), contents).is_err() {
            return;
        }
    }

    // Pass the canonical base so the confinement assertion in `fuzz::include` is
    // exact (a temp dir can sit behind a symlinked prefix such as macOS `/tmp`).
    let Ok(canonical_base) = fs::canonicalize(base) else {
        return;
    };
    _yamlrocks::fuzz::include(&canonical_base, root);
});
