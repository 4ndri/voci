# Rust conventions and review baseline

This is the project's baseline for Rust changes and reviews. Apply it to changed
code and its immediate dependencies. Existing code may predate these conventions;
fix nearby problems when useful, without turning a focused change into a global
style rewrite. Update this document when a project convention changes.

## Formatting and naming

- Use Rust 2024, as declared in `Cargo.toml`, and the default `rustfmt` style.
  Run `cargo fmt --all`; do not hand-align code or introduce custom formatter
  settings just to preserve a preferred layout. The
  [Rust Style Guide](https://doc.rust-lang.org/style-guide/) is the formatting reference.
- Use `UpperCamelCase` for types and traits, `snake_case` for modules, functions,
  and variables, and `SCREAMING_SNAKE_CASE` for constants. Follow the
  [Rust API naming guidelines](https://rust-lang.github.io/api-guidelines/naming.html):
  accessors normally omit `get_`, and setters use `set_`, such as `mode()` and
  `set_mode(mode)`.
- Prefer names that describe a value's role: `dialog`, `entry`, `selection`, and
  `request`. Short indices are fine in small loops. Avoid abbreviations that make
  a reader reconstruct the meaning from another function.
- Import production dependencies explicitly. `use super::*` is acceptable in
  private test modules. Keep one blank line between methods and logical items;
  use comments for invariants or reasons, not to narrate obvious statements.

## Modules, visibility, and responsibilities

- Split modules by responsibility, not by an arbitrary line limit. A file that
  combines terminal lifecycle, input policy, drawing, and asynchronous work needs
  a boundary even if it still fits on a few screens. A cohesive implementation
  does not need a new trait or abstraction just to make its file shorter.
- Keep helpers private. Use `pub(super)` for operations shared within a module
  family; use `pub(crate)` or `pub` only when the caller needs that scope. Rust's
  [visibility rules](https://doc.rust-lang.org/reference/visibility-and-privacy.html)
  allow child modules to access private parent state.
- Give each production module a short `//!` description. Describe contracts and
  surprising behavior with doc comments where useful. Avoid generic `utils`
  modules whose functions have no common owner.
- Keep a function at one useful level of abstraction. Extract a helper when it
  names a distinct policy or operation, particularly when an event dispatcher
  also implements mouse hit testing or a layout method constructs every hint.
  Do not enforce a fixed function-length cutoff on exhaustive Rust matches.
- Preserve a small external API. The TUI is private to the library and exposes
  its launcher and keybinding configuration only within the application; adding
  source files does not justify exposing application state.

The application uses feature-oriented modules in one package. Each directory has
an adjacent `.rs` facade; implementation modules stay private unless callers need
them. Do not restore the former root-level `provider`, `wikdict`, `coordinator`,
`completion`, `keybindings`, or `clipboard` modules as parallel implementations.

| Module | Responsibility |
| --- | --- |
| `src/main.rs` | Tokio entry point delegating to `cli::run` |
| `src/lib.rs` | Deliberate application, feature, configuration, and CLI facades |
| `src/domain.rs` | Shared language, dictionary query, and result contracts |
| `src/lookup/` | Lookup validation, language resolution, deadlines, provider contract and implementations |
| `src/history/` | Encounter models and SQLite events/queries |
| `src/app/lookup.rs` | Saved-result policy and recording fresh lookup attempts |
| `src/app/setup.rs` | Lazy provider construction, retry, and reuse across session clones |
| `src/app/outcome.rs` | Mapping lookup results/failures to persisted history outcomes |
| `src/config.rs` | File/environment/path loading with independent history and TUI settings |
| `src/text.rs`, `src/presentation.rs` | Shared sanitization and pure formatting; no terminal/clipboard I/O |
| `src/cli/` | Arguments, command adapters, completion, text/JSON output, and exit codes |
| `src/tui.rs` | Shell focus, effect types, and pane composition |
| `src/tui/lookup.rs` | `LookupPane`: draft, languages, results, suggestions, recent previews, and tab layout |
| `src/tui/history.rs` | `HistoryPane`: browsing, filters, selection, generations, and tab layout |
| `src/tui/history/dialog.rs` | History-owned filter dialog integrated with shell modal routing |
| `src/tui/details.rs` | `DetailsPane`: candidate selection, reading position, viewport, and rendering borrowed content |
| `src/tui/runtime.rs` | Terminal setup/restoration, background jobs, cancellation, and executing effects |
| `src/tui/events.rs`, `src/tui/navigation.rs` | Global input precedence and focus/navigation routing |
| `src/tui/render.rs` | Shell layout, footer hints, and shared widgets |
| `src/tui/input.rs` | Single-line editor adapter, graphemes, selections, and undo grouping |
| `src/tui/keybindings.rs`, `src/tui/clipboard.rs` | Key profiles/resolution and native clipboard ownership |

Dependencies follow these rules:

- CLI and TUI invoke application workflows for coordinated actions and history's
  read API for browsing, saved suggestions, and completion.
- Application code composes lookup, history, and configuration. `LookupPolicy`
  names saved/fresh behavior; the CLI prefers saved results unless `--fresh` is
  supplied, while TUI submission records a fresh attempt. Opening a saved entry
  remains distinct from submitting a new attempt.
- Lookup and history depend on domain contracts, not on each other's
  implementations or on frontends. `LookupRequest` is shared query data used by
  both lookup and history; it lives in domain and is re-exported by lookup.
- Domain has no Clap, Ratatui, HTTP, SQLite, configuration-loading, or process
  exit-code policy. Language parsing has a domain-owned error. Lookup errors,
  configuration errors, CLI exit codes, and frontend advice have separate owners.
- Configuration loads settings without constructing providers. Clap provider
  argument adaptation stays in CLI. Credentials remain out of debug output.
- The application maps results to history outcomes; history does not need to know
  lookup errors or frontend messages. Preserve stored codes and sanitized messages
  when changing that conversion. Shared formatting performs no I/O.
- The TUI shell owns global focus, event precedence, effects, and job lifecycles.
  Pane types own their state and transitions. Tab composition may coordinate
  panes, while Details receives borrowed content rather than reading shell state.

Keep feature unit tests beside their owner and cross-feature/CLI contracts in
`tests/`. Private TUI fixtures live under `src/tui/tests/`; runtime integration
coverage lives under `src/tui/runtime/tests.rs`. If a test launches itself in a
subprocess, derive its test path from `module_path!()` and verify that the child
actually ran a test after module moves.

Public Rust import paths can change in an architectural refactor, but persisted
JSON, database schema, command behavior, and keybinding configuration must remain
compatible. The binary and integration tests are separate crates: export the
entry points and feature contracts they need, not every implementation file.

## Types, ownership, and failure handling

- Use enums for mutually exclusive choices and named fields for multi-part
  requests. Avoid positional boolean tuples. `HistoryTarget` distinguishes the
  recent list from History; `HistorySelection` expresses first, last, or preserved
  selection. A boolean remains suitable for a single binary option such as the
  named `HistoryRead::oldest` field.
- Borrow data when ownership is unnecessary. Clone deliberately at asynchronous
  ownership boundaries or when a render operation needs a stable snapshot while
  updating view state. Do not obscure clear code with speculative allocation
  optimizations; measure before adding caches or complicated lifetimes.
- Use `Result` and `?` for fallible I/O. Keep storage/provider/clipboard errors
  visible to the user through the appropriate boundary. Do not add `unwrap()`
  for failures caused by input, configuration, storage, or the environment.
- An `expect()` is appropriate for a locally established internal invariant;
  its message should explain why the value must exist. Tests may use `unwrap()`
  for setup and assertions. Do not silently default away a broken invariant.
- Prefer ordinary matches, `if let`, and early returns over deeply nested
  branches. Use iterator chains when they make the transformation easier to read.

## TUI behavior that refactors must preserve

- State transitions request background work through `Effect`; rendering never
  starts provider/storage work. Drawing may update viewport state and mouse
  regions, which are rebuilt in paint order so overlays own their hit testing.
- Preserve input precedence: ignore key releases; handle Ctrl-C and Escape;
  then pane mode, insert mode, dialog controls, and normal navigation. Ordinary
  insert-mode letters must remain text, including when bindings are remapped.
- Preserve independent generations for lookup, history, recent entries, and
  suggestions. A stale response must not replace current data or its error state.
- Keep the draft, saved preview, selected candidate, reading position within a
  candidate, and viewport offset distinct. Long candidates must remain readable
  after navigation or resize.
- Perform a successful clipboard copy before cutting or changing a selection.
  Preserve grapheme boundaries and the existing undo grouping in both inputs.
- Keep terminal restoration and cancellation cleanup on exit/error paths.
  Provider setup remains lazy so saved history works with invalid provider settings.

## Validation and review checklist

Run the checks used by CI from the repository root:

```sh
cargo fmt --all -- --check
cargo clippy --locked --workspace --all-targets -- -D warnings
mise exec -- cargo test --workspace --locked
```

The test command uses mise so the installed GitVersion is available to the xtask
integration test. `cargo test --workspace --locked` is also useful, but without
GitVersion that integration test can skip itself locally; report that limitation.
The credentialed live-provider test remains explicitly ignored unless requested.
Use `cargo test --locked --lib tui::` for a focused TUI iteration.

Reviewers should check:

1. **Behavior:** Are edge cases, state transitions, asynchronous races, and
   failures preserved? Identify a concrete trigger and consequence for defects.
2. **Structure:** Does each module have a clear responsibility? Is visibility
   limited to actual callers? Are test fixtures kept out of production builds?
3. **Readability:** Are names idiomatic, choices typed, and invariants explained?
   Can request arguments be understood without consulting their declaration?
4. **Evidence:** Retain existing regression assertions when moving code. Add
   tests for meaningful changed behavior or uncovered risks, not for mechanical
   file moves or to mirror private implementation details. Prefer temporary
   storage, fixture providers, fake clipboards, and Ratatui's `TestBackend`.
5. **Reporting:** Separate correctness defects from maintainability suggestions.
   Include the reviewed scope/revision, symbol or file location, impact,
   resolution status, checks actually run, and any validation limits. Distinguish
   project preferences in this document from Rust language requirements.

Do not enable blanket pedantic lints or add allowances solely to settle a style
preference. A headless rendering test does not establish native clipboard or
terminal behavior, and a Linux test run does not establish macOS/Windows behavior.
