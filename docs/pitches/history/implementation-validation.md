# History implementation validation

Validated on Linux on 2026-09-19.

- `cargo test --locked --workspace`: 59 tests passed; the credentialed Microsoft live test remains ignored.
- `cargo clippy --locked --workspace --all-targets -- -D warnings`: passed.
- `cargo fmt --all -- --check`: passed.

History tests cover immutable start/outcome events, complete results, repeated attempts, Unicode and literal search, local-day boundaries across DST, stable paging in both directions, concurrent first launches and writers, lock contention, unsupported schemas, corrupt databases, and storage failure warnings. Subprocess tests verify that SIGINT records cancellation and hard termination leaves an unfinished attempt. CLI tests use isolated application directories and verify that history remains readable without configuration or provider setup.

TUI tests cover editing and shortcut isolation, Neo Noted aliases and pane directions, draft preservation during previews and filtering, stale completions, and rendering at wide, stacked, compact, and minimum sizes. Profile tests cover first-launch installation, preserving existing edits, and resolving paths beside the effective config file.

A manual session with the Neo Noted profile and installed WikDict data successfully looked up `Verbindlichkeit`, opened History, filtered for `verbindlich`, selected a translation with `n`, and copied it with `yy` to the local desktop clipboard. The lookup produced exactly one start and one outcome event. A separate PTY smoke check resized the TUI through 120×32, 80×24, 40×18, 8×3, and back to 120×32, then verified normal exit restored both terminal attributes and the alternate screen.

The updated Excalidraw draft was exported and visually inspected; its editable source and PNG now describe start/outcome events and unfinished attempts. The original TUI snapshot remains the comparison baseline.

macOS and Windows execution and native clipboard behavior were not verified locally; the existing CI matrix covers those platforms. The optional Microsoft live test was not run. The pitch's review of applicable Microsoft retention terms remains a release follow-up; these checks do not establish permission for long-term storage of Microsoft results.
