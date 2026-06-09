# Security Policy

Parsing untrusted input is the whole job of a YAML library, so security is a
first-class concern for YAMLRocks. This document explains how to report a
vulnerability, what is in scope, and the protections already built in.

## Supported versions

YAMLRocks is pre-1.0 and under active development. Security fixes land in the
latest released version, so the supported version is always the most recent
release (plus the `main` development branch). Please reproduce on the latest
release before reporting.

## Reporting a vulnerability

Please **responsibly** disclose security issues privately rather than opening a
public issue, so a fix can ship before the details are public. Use GitHub's
[private vulnerability reporting](https://github.com/frenck/yamlrocks/security/advisories/new)
for this repository.

Include, where possible:

- a description of the vulnerability and its impact;
- a minimal reproduction: the exact YAML input (or a small file) and the call
  that triggers it;
- the YAMLRocks version, Python version, and platform;
- a CVSS 3.1 vector string if you are comfortable scoring it.

What to expect:

- an acknowledgement within **7 days**, and updates as the issue is investigated
  and resolved;
- please allow at least **90 days** from your report before any public
  disclosure, to leave time for a fix and a coordinated release.

## Scope

In scope is anything that lets untrusted YAML compromise the host or the calling
process, for example:

- arbitrary code execution, or constructing unexpected Python objects, from
  parsing;
- memory-safety issues: the scanner, parser, resolver, and emitter are written in
  safe Rust, and `unsafe` is confined to the small, audited FFI layer at the PyO3
  boundary, so a soundness hole there is in scope;
- a crash, panic, hang, or unbounded memory or stack use on an input that should
  be rejected cleanly;
- reading or writing files or environment variables outside what the caller
  opted into through `OPT_INCLUDES` / `include_dir`.

Out of scope (please do not file these as vulnerabilities):

- output from automated scanners without a working proof of concept;
- purely theoretical attacks with no demonstrated impact;
- issues in third-party dependencies (report those upstream);
- resource use that stays within the documented, bounded limits (a large but
  finite input parsing slowly is not a vulnerability);
- anything requiring a caller to enable `OPT_INCLUDES` on input they do not
  trust, which the documentation explicitly warns against.

## Advisories and CVEs

For valid vulnerabilities at Medium severity or above, YAMLRocks publishes a
GitHub Security Advisory and requests a CVE through it. Lower-severity issues are
fixed in the normal release flow.

## Recognition

YAMLRocks is a community project and cannot offer bug bounties. Reporters of
valid issues are credited in the release notes and the advisory, unless they
prefer to remain anonymous.

## Safe by design

YAMLRocks never executes arbitrary code from YAML tags. There is no equivalent of
PyYAML's `yaml.load` with the default `Loader`. Application tags are dropped,
returned as `YAMLRocksTag` objects, or routed through an explicit `tag_handler` callback.
The PyYAML compatibility shim (`yamlrocks.compat`) maps `load`/`dump` onto the
safe variants.

## Threat model and protections

YAMLRocks is designed to parse untrusted YAML safely. The historical YAML parser
vulnerability classes are addressed as follows, with regression tests in
`tests/robustness/test_security.py`:

| Threat                                                                                                              | Status                                                                                                  |
| ------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------- |
| Arbitrary code execution via `!!python/object`/`!!python/name` tags (CVE-2017-18342, CVE-2020-1747, CVE-2020-14343) | Not possible: YAMLRocks cannot construct Python objects from tags; such payloads return inert data.     |
| "Billion laughs" alias-expansion memory bomb                                                                        | Bounded: alias expansion is charged against a node budget and rejected before allocating.               |
| Stack exhaustion from deeply nested input                                                                           | Bounded: a maximum nesting depth is enforced in both the fast-path decoder and the round-trip composer. |
| Infinite recursion from circular `!include` files                                                                   | Detected and rejected.                                                                                  |
| Path traversal or symlink escape through `!include`/`!secret`                                                       | Confined: included paths are resolved and must stay within `include_dir`.                               |
| Invalid UTF-8 / malformed input                                                                                     | Rejected with a clean `YAMLRocksDecodeError` (a `ValueError`); never crashes the interpreter.           |

The full YAML test suite (300+ cases) plus the property and coverage-guided fuzz
tests (`cargo fuzz`, ClusterFuzzLite) confirm that no input crashes or hangs the
process: malformed input is rejected with an error, never a panic.

## Supply chain

Release artifacts are built and published from CI with verifiable provenance,
which is also what a downstream integrator's due diligence (for example under the
EU Cyber Resilience Act) looks for:

- **Trusted Publishing**: wheels and the source distribution are uploaded to PyPI
  over OIDC, so no long-lived PyPI token is stored in this repository.
- **Embedded SBOM**: every wheel ships a CycloneDX SBOM of the Rust dependency
  tree at `<name>.dist-info/sboms/yamlrocks.cyclonedx.json`, so the exact crates
  and versions a wheel was built from can be recovered from the artifact itself.
- **Signed provenance**: build provenance and a CycloneDX SBOM for the release
  are attested with Sigstore (GitHub Artifact Attestations).

Verify a downloaded wheel or sdist against its attestation with the GitHub CLI:

```bash
gh attestation verify ./yamlrocks-<version>-<tag>.whl --repo frenck/yamlrocks
```

Artifacts are immutable once published: a fix is always a new release, never an
edit to an existing one.

## Notes for callers

- `!include`, `!secret`, and `!env_var` are opt-in (`OPT_INCLUDES`) and read the
  filesystem and environment. Only enable them for configuration you trust, and
  scope `include_dir` to the intended directory.
- Resource limits (the node and depth bounds) are conservative defaults sized
  far above real documents; they exist to contain pathological input.
  </content>
