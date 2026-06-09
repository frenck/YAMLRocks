---
title: Security
description: YAMLRocks's safety model for parsing untrusted YAML.
---

YAMLRocks is designed so that calling `loads` on untrusted YAML is safe by default.
There is no opt-in "safe loader" to remember to choose, the way PyYAML makes you
pick `safe_load` over `load`. The fast path does not construct arbitrary Python
objects, and the parser enforces hard limits that contain pathological input.

## Threat model

YAMLRocks assumes the YAML text may be fully attacker-controlled and that the
attacker's goals are code execution, denial of service (memory or CPU
exhaustion), or crashing the interpreter. The mitigations below cover each.

| Attack                                                            | Mitigation                                                                                  |
| ----------------------------------------------------------------- | ------------------------------------------------------------------------------------------- |
| Arbitrary object construction (`!!python/object/apply:os.system`) | No constructor exists; tags never instantiate Python objects.                               |
| "Billion laughs" alias bomb                                       | Alias expansion is charged against a node budget and rejected before the copy is allocated. |
| Deeply nested input (`[[[[...`)                                   | A maximum nesting depth is enforced in both decode paths.                                   |
| Circular `!include` files                                         | Include cycles are detected and rejected with a clear error.                                |
| Malformed or invalid UTF-8                                        | Rejected with `YAMLRocksDecodeError`; never crashes.                                        |

## No code execution

Unlike PyYAML's `yaml.load` with the default loader, YAMLRocks has no mechanism to
construct arbitrary Python objects from tags. A payload such as
`!!python/object/apply:os.system [...]` returns inert data; nothing is executed.
The historical PyYAML remote-code-execution issues (CVE-2017-18342,
CVE-2020-1747, CVE-2020-14343) do not apply, because the code path they exploit
does not exist here.

Application tags are dropped by default, surfaced as `YAMLRocksTag` objects with
`OPT_PASSTHROUGH_TAG`, or routed through your own `tag_handler` callback. You stay
in control of how a tag is interpreted, and you only interpret the tags you opt
into:

```python
import yamlrocks

# An unrecognized tag yields inert data, never a constructed object.
yamlrocks.loads(b"value: !!python/object/apply:os.system ['echo hi']")
# {'value': ['echo hi']}
```

## Denial-of-service protections

A small YAML document can describe an enormous data structure through nested
aliases (the "billion laughs" attack) or through extreme nesting. YAMLRocks bounds
both. The limits are sized far above any realistic document, so they only ever
trigger on input that is trying to exhaust your memory or stack.

An alias bomb is rejected with a `YAMLRocksDecodeError` rather than being expanded:

<!-- verify: skip -->

```python
import yamlrocks

bomb = b"""
a: &a ["x","x","x","x","x","x","x","x","x"]
b: &b [*a,*a,*a,*a,*a,*a,*a,*a,*a]
c: &c [*b,*b,*b,*b,*b,*b,*b,*b,*b]
d: &d [*c,*c,*c,*c,*c,*c,*c,*c,*c]
e: &e [*d,*d,*d,*d,*d,*d,*d,*d,*d]
f: &f [*e,*e,*e,*e,*e,*e,*e,*e,*e]
g: &g [*f,*f,*f,*f,*f,*f,*f,*f,*f]
h: &h [*g,*g,*g,*g,*g,*g,*g,*g,*g]
"""

yamlrocks.loads(bomb)
# yamlrocks.YAMLRocksParseError: document expands to too many nodes
#                         (possible alias bomb) at line 9, column 8
```

Deeply nested input hits the depth cap instead of overflowing the stack:

<!-- verify: skip -->

```python
import yamlrocks

yamlrocks.loads(b"a: " + b"[" * 5000 + b"]" * 5000)
# yamlrocks.YAMLRocksDecodeError: maximum nesting depth (1000) exceeded ...
```

The full YAML test suite plus fuzz testing confirm that no input crashes
or hangs the interpreter; a broken document always returns a `YAMLRocksDecodeError`,
never a segfault or an infinite loop. A dedicated differential fuzz target goes
further, checking that data is never silently corrupted in transit: anything
`loads` accepts must survive a `dumps`/`loads` round-trip with identical values.

## Includes and secrets

`!include`, `!secret`, and `!env_var` read the filesystem and environment, so
each is off unless you opt in, and each has its own flag: `OPT_INCLUDES` for the
`!include` family, `OPT_SECRETS` for `!secret`, and `OPT_ENV_VAR` for `!env_var`.
The flags are independent on purpose: enabling includes does not grant a
document access to your secrets store or environment. Turn on only what a given
input is trusted to reach, and scope `include_dir` to the directory you intend.
Include cycles (file A includes B includes A) are detected and rejected rather
than followed forever. In round-trip mode, `!secret` and `!env_var` directives
are preserved on output, so a resolved secret value is never written back into a
file on disk.

Every path the include resolver touches is confined to `include_dir`. A target
that climbs out with `..`, an absolute path, or a symlink whose real location
lands outside the directory is rejected, and that check applies equally to files
discovered through `!include_dir_*` and to the `secrets.yaml` that `!secret`
searches for. A symlink planted inside the configuration tree cannot be used to
read `/etc/passwd` or anything else beyond the base.

`!env_var` is different in kind: when `OPT_ENV_VAR` is on, the document chooses
which environment variable to read by name, with no allowlist or prefix
restriction. A document you load can therefore surface any variable in the
process environment, including cloud credentials, tokens, and database
passwords. Enable `OPT_ENV_VAR` only for configuration you fully trust, and
prefer scrubbing or namespacing the environment before loading semi-trusted
input.

## Reporting

The repository's
[`SECURITY.md`](https://github.com/frenck/yamlrocks/blob/main/SECURITY.md) is the
single source of truth for the security policy: what is in scope, response
timelines, supported versions, and how advisories and CVEs are handled. Please
read it before reporting.

In short: disclose privately rather than opening a public issue, so a fix can
ship before the details are public. Use GitHub's
[private vulnerability reporting](https://github.com/frenck/yamlrocks/security/advisories/new)
for this repository, and where possible include a minimal reproduction (the exact
YAML input and the call that triggers it) plus the YAMLRocks version, Python
version, and platform. You can expect an acknowledgement within 7 days; please
allow at least 90 days before any public disclosure. Valid issues are credited in
the release notes and advisory unless you prefer to stay anonymous, and those at
Medium severity or above get a published GitHub Security Advisory with a CVE.

Parsing untrusted input is the whole job of a YAML library, so reports are taken
seriously: a crash, panic, hang, or unbounded memory or stack use on input that
should be rejected cleanly is in scope, as is any memory-safety issue at the
`unsafe` FFI boundary or any escape from the opt-in `OPT_INCLUDES` / `include_dir`
sandbox. See `SECURITY.md` for the full scope and out-of-scope list.

## See also

- [Loading YAML](/guides/loading/): the parsing entry points and their defaults.
- [Includes](/guides/includes/): the opt-in `!include` resolver and `include_dir`.
- [Custom tags](/guides/tags/): `tag_handler` and `OPT_PASSTHROUGH_TAG`.
- [Exceptions](/reference/exceptions/): the `YAMLRocksDecodeError` these guards raise.
