<!--
  You're awesome, thanks for contributing to YAMLRocks!

  Please do not delete the sections of this template. Fill in what applies and
  write "None" where it does not; the structure helps reviewers process your
  pull request quickly.
-->

## Breaking change

<!--
  Does this change behavior for existing users (an API change, a different parse
  or emit result, a removed or renamed option)? If so, describe what breaks, how
  to adapt, and why the change is worth it. Write "None" if nothing breaks.
-->

## Proposed change

<!--
  Describe the change and, just as important, why it should be accepted. Link the
  issue or discussion it addresses so reviewers have the full picture.
-->

## Type of change

<!-- Put an `x` in the one box that best describes this PR. -->

- [ ] Dependency or tooling upgrade
- [ ] Bugfix (non-breaking change that fixes an issue)
- [ ] New feature (non-breaking change that adds functionality)
- [ ] Deprecation (replaces or removes a feature, with a migration path)
- [ ] Breaking change (a fix or feature that changes existing behavior)
- [ ] Code quality, refactor, or test-only change
- [ ] Documentation only

## Additional information

<!-- Details help reviewers. Remove the lines that do not apply. -->

- This PR fixes or closes issue: fixes #
- This PR is related to:
- Link to a separate documentation pull request:

## Checklist

<!-- Put an `x` in the boxes that apply. -->

- [ ] I have read the [AI Policy](../AI_POLICY.md), and this pull request was not created by an autonomous agent.
- [ ] I fully understand the code in this pull request and can explain every line, including any AI-assisted changes.
- [ ] The change is covered by tests, and `uv run pytest` passes locally. **A pull request cannot be merged unless CI is green.**
- [ ] `uv run ruff check .` and `uv run ruff format --check .` pass.
- [ ] `cargo fmt --check` and `cargo clippy --all-targets -- -D warnings` pass.
- [ ] Round-trip fidelity is preserved: an unmodified document still re-emits byte-for-byte.
- [ ] No commented-out or dead code is left in the pull request.

If the change is user-facing:

- [ ] Documentation under `docs/` is added or updated, and `docs/verify_examples.py` still passes.

<!--
  Reviewer time is the scarcest resource on most projects. If you have a moment,
  reviewing one or two other open pull requests is a great way to help out, and a
  good way to get to know the codebase.
-->
