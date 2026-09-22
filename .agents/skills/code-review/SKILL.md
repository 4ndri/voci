---
name: code-review
description: Review Rust code, diffs, commits, branches, or pull requests in this repository for defects and maintainability using its Rust conventions. Save a Markdown report under ./.reviews/{date}_code-review/{title}.codereview.md. Use when the user requests a Rust code review.
---

# Rust Code Review

Review this repository's Rust code and save the findings as a Markdown artifact.
Include Cargo configuration and related tests when they affect the reviewed Rust
behavior.

## Scope and evidence

- Use [docs/rust-conventions.md](../../../docs/rust-conventions.md) as the review
  baseline.
  That document is the shared baseline for Rust style, module responsibilities,
  TUI invariants, compatibility, and validation. Apply it to the reviewed code
  and its immediate dependencies; distinguish project preferences from Rust
  language requirements. Avoid unrelated repository-wide style rewrites.
- Use the user's requested scope and comparison base. If unspecified, review
  staged and unstaged changes, including relevant untracked source files. If
  there are no local changes and no clear target, ask which revision or area to
  review. Record the exact scope, HEAD revision, comparison base when applicable,
  and whether uncommitted changes were included.
- Read surrounding code and relevant callers and tests to establish behavior.
  For a diff review, focus findings on problems introduced or exposed by the
  change; label relevant pre-existing issues separately.
- Prioritize correctness, regressions, data integrity, security, and failure
  handling. Include maintainability suggestions when grounded in the reviewed
  code or repository conventions, separately from correctness defects.
- Each finding needs a concrete trigger, consequence, and precise repository
  path with line numbers or symbol. Explain the evidence and a suggested remedy.
  Mark uncertainty explicitly; put unverified concerns in open questions rather
  than presenting them as established defects.
- Run relevant checks required by repository guidance when feasible. Use
  non-mutating checks, and record commands, outcomes, skips, and limitations.
  Do not imply that static inspection or passing tests establish untested behavior.
- A review does not authorize implementing fixes, changing source files, or
  posting comments to an external service. Report each finding's observed
  resolution status; leave fixes for a requested implementation task.

## Rust review focus

Apply these checks where relevant to the scope, using the conventions document
for this repository's specific contracts:

- Check ownership and borrowing at task boundaries, deliberate cloning,
  visibility, typed state, and module dependencies. Recommend abstractions or
  allocation changes only when they solve a concrete problem.
- Trace fallible operations through `Result` propagation and frontend reporting.
  Verify that `unwrap()` and `expect()` rely on established internal invariants,
  not user input or environmental assumptions. Check any unsafe code for the
  safety contracts required by its callers and implementation.
- For async code, inspect cancellation, timeouts, lock lifetimes, blocking work
  on runtime threads, and stale responses. For TUI changes, check the documented
  generation, input precedence, clipboard, Unicode, and terminal cleanup rules.
- Check persisted schema and serialization compatibility, CLI behavior, and
  configuration contracts. Evaluate public API and dependency changes against
  actual callers and the repository's supported toolchain and platforms.
- Inspect regression assertions and meaningful edge-case coverage. Do not treat
  compiler or Clippy success as proof of correct application behavior, or invent
  findings solely to satisfy a stylistic preference.

## Validation

Follow the current validation commands in `docs/rust-conventions.md`. The baseline
checks, run from the repository root, are:

```sh
cargo fmt --all -- --check
cargo clippy --locked --workspace --all-targets -- -D warnings
mise exec -- cargo test --workspace --locked
```

Use `cargo test --locked --lib tui::` for focused TUI checks when useful. Record
any checks not run and why. The plain Cargo workspace test command can skip the
GitVersion-dependent integration test; disclose that limitation if used. Keep
credentialed live-provider tests ignored unless requested. Do not run formatting
or automatic fixes that modify reviewed source files. Headless tests do not
establish native terminal or clipboard behavior, and one platform's results do
not establish cross-platform behavior.

## Save the report

- Resolve `./` to the reviewed repository or worktree root.
- Use the current local date in `YYYY-MM-DD` format for `{date}`.
- Derive `{title}` from the user-provided title or a concise description of the
  review scope. Convert it to a lowercase kebab-case filename using letters,
  digits, and hyphens only; use `code-review` if the result is empty.
- Create `./.reviews/{date}_code-review/` as needed and write
  `{title}.codereview.md`, even when there are no findings. If that file exists,
  append `-2`, `-3`, and so on to the title unless the user requested updating it.
- Keep the report useful without the conversation. Include:
  - A descriptive title, review date, scope, and reviewed revision/base.
  - A brief assessment supported by the findings and validation evidence.
  - Correctness findings ordered by severity: critical, high, medium, or low.
    For each, include location, trigger, impact, evidence, suggested remedy, and
    resolution status. State explicitly when no actionable defects were found.
  - Maintainability suggestions, if any, separately from defects.
  - Open questions and assumptions, if any.
  - Checks actually run, their results, and remaining validation limits.
- Do not change ignore rules or commit the report unless requested. Preserve
  existing reviews and unrelated working-tree changes.

End with a short summary of the most important findings and a clickable link to
the saved report. A chat-only review does not complete this workflow.
