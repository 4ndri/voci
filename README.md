# voci

Quick German ↔ English dictionary lookup in your terminal. WikDict is the default: no account or API key required.

```sh
voci Verbindlichkeit
voci liability
voci --from de --to en Verbindlichkeit
voci shell
```

The CLI prints a word, its language direction, and up to eight translation candidates. The TUI keeps the same lookup loop open for repeated searches. Every valid lookup attempt is saved locally, including repeats, misses, failures, and cancellations. Browse saved encounters from either interface; flashcard practice remains a future feature.

## Install

Download the archive for your operating system and CPU from [GitHub Releases](https://github.com/4ndri/voci/releases). Extract it and put `voci` (or `voci.exe` on Windows) in a directory on your `PATH`. One executable provides both the CLI and `voci shell`; Rust is not needed to run it. Release archives include documentation, and `SHA256SUMS` provides checksums for each download.

Available release targets are Linux x64 (GNU libc, built on Ubuntu 24.04; use glibc 2.39 or newer), macOS Intel and Apple Silicon, and Windows x64. Binaries are currently unsigned and macOS builds are not notarized. Dictionary files are downloaded separately on first lookup.

### Build from source

Install a current stable Rust toolchain and [mise](https://mise.jdx.dev/), then run from a full Git checkout:

```sh
mise install
mise run install
```

The [mise file tasks](https://mise.jdx.dev/tasks/file-tasks.html) launch the private Rust `xtask` workspace member, which Cargo compiles automatically on first use. GitVersion is pinned in `mise.toml`; Rust remains supplied by your toolchain. Python is not required. On Windows, the mise launchers also require Bash (provided by Git for Windows). The install task builds a release binary with the GitVersion version and installs or updates `voci` in Cargo's installation directory (normally `~/.cargo/bin`, or `$CARGO_HOME/bin` when set). Ensure that directory is on your `PATH`; then both `voci Verbindlichkeit` and `voci shell` work from any directory. No `sudo` is needed. Rerun the task after changing the code to update the installed application.

Without mise or Git history, Cargo can still build and install the application using the fallback version in `Cargo.toml`:

```sh
cargo install --path . --locked --bin voci --force
```

Or use `cargo run --locked -- Verbindlichkeit` / `cargo run --locked -- shell` during development. The implementation uses Clap, Ratatui, and its Crossterm backend. CI is configured for Linux, macOS, and Windows.

### Development and release tasks

The same tasks run locally and in GitHub Actions. They are also available directly as `cargo xtask <task>` when GitVersion is on `PATH` (or through `mise exec -- cargo xtask <task>`). Regular `cargo run`, `cargo build`, and `cargo test` still default to the `voci` application.

| Command | Behavior |
| --- | --- |
| `mise run version` | Print GitVersion's `SemVer` for the current checkout. |
| `mise run build` | Build an optimized release executable with that version. |
| `mise run package` | Build, then create a versioned archive and `.sha256` checksum in `dist/`. |
| `mise run run -- Verbindlichkeit` | Build and run a development binary. |
| `mise run run -- shell` | Start the interactive TUI with terminal input/output attached. |
| `mise run test` | Run the Rust test suite in the development profile. |
| `mise run install` | Install or update the versioned executable for the current user. |

Build, package, run, test, and install accept `--target <Rust triple>`, `--profile dev|release`, `--target-dir <directory>`, `--offline`, and `--jobs <count>`. The default target is the Rust host triple. All Cargo commands use `--locked`. Cross-compilation requires installing the target and its linker/toolchain separately; release jobs build natively on each platform. The mise launchers apply `--offline` to both the `xtask` bootstrap and the application build; install mise tools and fetch Cargo dependencies first. For direct invocation, use `cargo --offline xtask build --offline` to cover both stages.

```sh
mise run build --profile dev --offline
mise run package --target x86_64-unknown-linux-gnu --output-dir dist
mise run package --target x86_64-unknown-linux-gnu --no-build
mise run run -- --from de --to en 'ice cream'
mise run test --test cli --filter help -- --nocapture
mise run test --lib
mise run install --root /tmp/voci-install
mise run build --help
mise run run --task-help
```

Arguments after `--` go unchanged to the app or Rust test harness. Paths supplied as task arguments are relative to the task's working directory. Build outputs go to `target/<triple>/<profile>/` (`dev` uses the `debug` directory), or the supplied `--target-dir` / `CARGO_TARGET_DIR` directory. Package's `--no-build` requires a matching build receipt: version, commit, target, profile, and executable checksum must agree. It packages that exact binary; omit the flag to include new source edits. Development archives have a `-dev` suffix to distinguish them from releases. Archives include the executable, documentation, attribution, and `build-info.json`.

GitVersion configuration lives in `GitVersion.yml` and uses GitHub Flow with Conventional Commit bump rules (`feat`, `fix`/`perf`, and breaking changes). GitVersion needs full history and tags; a shallow clone fails with recovery instructions. The `SemVer` is embedded at compile time through `VOCI_BUILD_VERSION` and used in archive names. **Neither `Cargo.toml` nor `Cargo.lock` is rewritten or committed by these tasks.** The manifest's version remains Cargo's package metadata and the fallback for direct Cargo builds; publishing to crates.io would require updating it separately. Cargo automatically rebuilds when the compile-time version changes.

For run/test task options, use `--task-help`; `--help` is forwarded to the app or test harness. Run the task implementation checks with `mise exec -- cargo test --locked --package xtask`. CI formats and lints the entire workspace, and runs both application and tooling tests. The tooling tests cover archives, checksums, stale builds, argument forwarding, Cargo failures, and real GitVersion tag calculation.

## Publishing a release

The release workflow runs automatically on pushes to `main`; CI continues to run on pull requests. Merge the desired changes to publish. No manual tag or Cargo version bump is needed.

The workflow fetches full Git history and calculates GitVersion's `SemVer` before building. Every platform verifies that its calculated version matches that shared version. Publishing creates the corresponding `v<SemVer>` tag at the exact commit that triggered the workflow, even if `main` has advanced in the meantime, and uses that tag as the release title (for example, `v0.1.0`). Existing release tags remain part of GitVersion's history for calculating subsequent versions. Use `mise run version` to preview the calculated version locally. The `main` branch uses GitVersion's `ContinuousDeployment` mode to produce stable versions without a prerelease commit counter; local feature branches receive prerelease labels.

Each platform calls `mise run test`, `mise run build`, and `mise run package --no-build`, and verifies that the executable reports the GitVersion version before packaging it. After every build succeeds, the workflow generates combined SHA-256 checksums and release notes, uploads all assets to a draft, and publishes it. It uses GitHub's built-in token; no publishing secret is required. Failed uploads leave a draft that can be completed by rerunning the workflow. Published releases are never overwritten; merge fixes to `main` for GitVersion to calculate the next release.

## WikDict: first run and offline lookup

The first lookup automatically downloads the two German/English databases from [WikDict](https://www.wikdict.com/page/download), about 45 MiB total, and builds a local lookup index. Download progress goes to stderr; Ctrl-C cancels setup. Later lookups use the installed files without contacting any service. The TUI prepares dictionaries only when a lookup is submitted, so history remains usable without them.

The data release is pinned to `2_2026-06`. Downloads have a 120-second deadline per file and are validated before installation. Failed or cancelled transfers never replace a completed dictionary. A retry downloads only missing files; there are no background updates.

| Platform | Dictionary directory |
| --- | --- |
| Linux | `$XDG_DATA_HOME/voci/wikdict/2_2026-06`, or `~/.local/share/voci/wikdict/2_2026-06` |
| macOS | `~/Library/Application Support/voci/wikdict/2_2026-06` |
| Windows | `%LOCALAPPDATA%\voci\data\wikdict\2_2026-06` |

The directory contains `de-en.sqlite3` and `en-de.sqlite3`. Override its parent with `[wikdict].data_dir` in the configuration file. A corrupt installed file produces a recovery message; remove that file and run another lookup to fetch a fresh copy. For a machine that is already offline, copy these files from another voci installation into the versioned directory first.

Lookup matches Unicode case and canonical accent variants, including `STRASSE` → `Straße` and `GRÜSSE` → `Grüße`. Existing dictionaries receive a one-time local index upgrade on the next run; no download is needed, and original dictionary records are preserved. This small provider uses the translation databases, not WikDict's much larger monolingual/inflection databases: some inflected forms may need their base word. Candidate ordering and sense descriptions come from WikDict; voci does not invent additional meanings.

Dictionary data is by Karl Bartel/WikDict, derived from Wiktionary contributors via DBnary, under [CC BY-SA 4.0](https://creativecommons.org/licenses/by-sa/4.0/). Results include attribution. Original dictionary records are preserved; voci adds a local lookup index and selects/formats candidates for display. See [dictionary attribution](docs/pitches/lookup/wikdict-attribution.md) for source and license links.

## Optional Microsoft provider

Select Microsoft with `--provider microsoft`, or set `provider = "microsoft"` in your configuration. An Azure Translator subscription key is required only for this provider. Create or use a Translator resource in your own Azure subscription and obtain its key from **Keys and Endpoint**. Regional and multi-service resources also need their Azure region; a global Translator resource does not require a region header. See Microsoft's [authentication documentation](https://learn.microsoft.com/en-us/azure/ai-services/translator/text-translation/reference/authentication).

Set `VOCI_MICROSOFT_KEY` in the process environment using your preferred secret-management mechanism. For example, enter it without echo or shell-history storage in Bash:

```bash
read -rsp 'Translator key: ' VOCI_MICROSOFT_KEY
export VOCI_MICROSOFT_KEY
```

In PowerShell 7:

```powershell
$env:VOCI_MICROSOFT_KEY = Read-Host 'Translator key' -MaskInput
```

The key is never accepted as a CLI flag or configuration-file field. `VOCI_MICROSOFT_REGION` can override the region in the configuration file. Requests go directly to Microsoft's public HTTPS endpoint; private endpoints and Entra authentication are not supported in this slice.

With Microsoft selected, each lookup sends the query and language pair to Microsoft. Automatic source resolution makes **two concurrent dictionary requests**; `--from` makes one. These requests consume your subscription's quota. voci performs no automatic retries and adds no query telemetry. The whole lookup has a ten-second deadline, with a three-second connection timeout.

## Configuration

Configuration is optional. Create a `config.toml` at the applicable location:

| Platform | Default location |
| --- | --- |
| Linux | `$XDG_CONFIG_HOME/voci/config.toml`, or `~/.config/voci/config.toml` |
| macOS | `~/Library/Application Support/voci/config.toml` |
| Windows | `%APPDATA%\voci\config\config.toml` |

```toml
provider = "wikdict"
target_language = "en"

[history]
# Optional; relative paths are resolved beside this config.toml:
# database = "data/voci.db"

[wikdict]
# Optional directory containing the versioned dictionary folder:
# data_dir = "/path/to/dictionaries"

[microsoft]
# Omit for a global resource. Otherwise use your resource's actual region.
# region = "westeurope"
```

Use `--config PATH` to read another file. A missing default file uses defaults; a missing explicit file is an error. `VOCI_MICROSOFT_REGION` takes precedence over the file. The preferred target defaults to English. `--provider` overrides the configured provider; the default is `wikdict`. Microsoft environment variables are ignored when WikDict is selected.

## Language handling

With no `--from`, voci checks both German → English and English → German in the selected provider (locally for WikDict). One matching direction determines the source; two matches ask you to select a source; two misses ask you to check spelling or supply a source. A failed request never counts as a dictionary miss. This is an inference from dictionary coverage, not a general language detector.

Explicit `--from` and `--to` selections override defaults. When no target is specified and the preferred target equals the source, voci chooses the other supported language. An explicitly selected target is never changed.

```sh
voci --from de Gift
voci --from en Gift
voci -- shell                 # Look up the literal word “shell”
voci 'ice cream'              # A quoted dictionary expression; no translation fallback
```

Only `de` and `en` are supported. French and additional providers are future extensions. Use `--provider wikdict` or `--provider microsoft` with either the CLI or `shell`. A missing word stays a dictionary miss; it is never silently machine-translated. Use `voci --help` for the supported flags. Running `voci` with no arguments prints help.

## History

```sh
voci history
voci history --today --limit 20
voci search verbindlich
voci search STRASSE --today
```

History lists encounters newest first (20 by default). Search matches saved queries and individual translations with Unicode-aware, case-insensitive literal substring matching. `--today` uses the current local calendar day; filters apply before the limit. Repeated lookups remain separate, and all saved translation candidates are available, including candidates omitted from concise live CLI output.

Every valid submission records a start and then an outcome. Misses, ambiguous/undetermined languages, provider setup failures, and cancellations remain visible. Invalid queries and invalid language pairs are rejected before recording. An attempt with no saved outcome is shown as **unfinished**, for example after a forced termination; this does not imply that it failed or completed. Browsing, filtering, previewing, and copying do not create attempts or contact providers.

| Platform | History database |
| --- | --- |
| Linux | `$XDG_DATA_HOME/voci/voci.db`, or `~/.local/share/voci/voci.db` |
| macOS | `~/Library/Application Support/voci/voci.db` |
| Windows | `%LOCALAPPDATA%\voci\data\voci.db` |

The database is created on the first valid submission and is independent of dictionary files, working directory, and `[wikdict].data_dir`. It stores queries, language selections, provider identity, timestamps, outcomes, full successful results, and attribution. This is persistent personal data; credentials and raw provider responses are not stored. SQLite may also create `-wal` and `-shm` sidecars. Do not delete or replace this database when recovering dictionary downloads.

Override the database with `[history].database` in `config.toml`, or `--database PATH` on any lookup, `shell`, `history`, or `search` command. The flag takes precedence; relative flag paths use the working directory, while relative config paths use the config file's directory. History commands read only the history settings and never initialize a provider. Unreadable or malformed history configuration produces an error rather than silently selecting a different database; `--database` can bypass that configuration for history reads.

```sh
voci --database /tmp/voci-test.db shell
voci --database /tmp/voci-test.db history
```

The default filename is now `voci.db`. An existing `history.sqlite3` is left intact and is not migrated or merged automatically. To keep using it, set `[history].database` to its path or pass `--database /path/to/history.sqlite3`.

An absent database is normal empty history. Storage problems show the database location and a warning while retaining useful lookup output; they do not change the lookup exit status. A missing final write can leave an unfinished entry. History commands themselves report an unreadable database as an error. Use `voci -- history` or `voci -- search` to look up those literal words.

## TUI

`voci shell` requires an interactive terminal on stdin and stdout. Optional `--from`, `--to`, `--provider`, `--config`, and `--database` flags work before or after `shell`. The app opens on **lookup** with the word field ready for typing. Source/target choices and unfinished input remain local to the session.

**Lookup** keeps the word, language selectors, and current result, with up to five recent encounters at the bottom. Selecting a recent encounter previews its stored result or failure in the details area without modifying the input. Escape leaves a saved preview. Submitting a word performs a new lookup and records a new attempt.

**History** shows paged encounters and their full details. Wide terminals place details to the right; narrower terminals stack them below. Small terminals show the focused list or details pane. Selection and filters survive tab switches. `/` opens a filter dialog for text and All history/Today; Apply commits changes, Clear resets its fields, and Cancel preserves the previous filters. `Ctrl-r` refreshes history and retries a storage read after a problem is resolved.

Inputs have **NORMAL**, **INSERT**, and **VISUAL** modes. In normal mode, `Enter` or `i` enters insert mode at the cursor; `a` enters insert mode after the current character (a whole Unicode grapheme), or at the end if already there; `Enter` in insert mode submits the lookup or applies the filter. Letter shortcuts are ordinary characters in insert mode. In normal mode, Left/Right and configured direction keys move the cursor, and Home/End bindings move to either end. `p` inserts clipboard text at the cursor without submitting. Multiline/control-character paste is rejected without changing the field.

`v` starts character selection; move with arrow/profile keys, word motions, or Home/End, then `y` copies, `d`/`x` cuts, `c` cuts and enters insert mode, or `p` replaces the selection from the clipboard. In normal mode, `d` and `x` cut the character under the cursor. The desktop clipboard serves as the shared register for cut/copy/paste; a cut at the end of an empty field does nothing. Selection respects Unicode graphemes, including combining accents. `v` or Escape leaves visual mode. Copy/cut failures preserve the selection and text. These are small Vim-style editing modes; operator sequences and named registers are not implemented.

`b` / `Ctrl-Left` jumps back to a word beginning; `e` / `Ctrl-Right` jumps forward to a word end. Ctrl-arrow word motions also work in insert mode, where the caret lands after the word's last character. Normal/visual word-end motions land on the last character, so visual selections include it. Words group Unicode letters, digits, and underscores; punctuation forms separate runs and whitespace separates them. In Neo Noted inputs, `b` means word-begin; `Home` and `gg` still reach the start of the field, and `b` retains its Home action in lists.

`u` undoes and `Ctrl-r` redoes in normal input mode. Each insert session is one undo step; normal-mode cuts, pastes, and visual replacements are separate steps. A visual `c` change and its subsequent typing undo together as one step. If copying the selection fails, `c` leaves the text, selection, and visual mode intact. Up to 100 steps are kept per input, and a new edit after undo discards redo. Undo/redo restore text and cursor without changing the clipboard or saved lookup history. Reopening the filter starts from its applied text with a fresh undo history. In the history list, `Ctrl-r` still refreshes results.

Normal and visual inputs use a steady block cursor; insert inputs use a steady I-beam. Voci resets the terminal's default cursor shape on exit and panic. Cursor-shape support depends on the terminal.

`Ctrl-w` toggles persistent **PANE** mode, including from insert mode. Arrow keys and configured direction keys then change focus without modifying fields. Pane mode stays active across repeated moves and pauses until Enter, Escape or `Ctrl-w`; leaving it returns to normal mode. Inside the filter dialog it moves between the dialog's controls. The dialog uses the same bordered text fields and cyan focus borders as Lookup, with separate Date, Apply, Clear, and Cancel controls. Enter leaving pane mode only confirms focus; press it again to edit or activate the selected control.

Escape first leaves pane mode, a pending key sequence, or a text-editing mode. In a filter text field, the first Escape returns to normal mode and another closes the dialog. Existing lookup cancellation and saved-preview dismissal remain available. The status area shows the current mode and active bindings.

| Default keys | Action |
| --- | --- |
| `gt` / `gT` | Next / previous tab |
| `Tab` / `Shift-Tab` | Next / previous control or pane |
| `Ctrl-w`, then arrows or configured directions | Enter persistent pane selection; `Enter`, `Esc` or `Ctrl-w` exits |
| Arrow keys or `h/j/k/l` | Navigate controls, entries, or candidates |
| `Home` / `gg`, `End` / `G` | First / last matching entry or candidate |
| `PageUp` / `PageDown` | Move five entries or candidates |
| `i`, or `Enter` on an input in normal mode | Enter insert mode |
| `u` / `Ctrl-r` on an input in normal mode | Undo / redo input edits |
| `a` on an input in normal mode | Enter insert mode after the current character |
| `p` in an input in normal mode | Paste clipboard text at the cursor |
| `b` / `Ctrl-Left`, `e` / `Ctrl-Right` | Previous word beginning / next word end in text fields |
| `Ctrl-Left` / `Ctrl-Right` in insert mode | Move the insertion caret by word |
| `d` / `x` in normal or visual input mode | Cut the current character or selection to the clipboard |
| `c` in visual input mode | Cut selection to clipboard and enter insert mode |
| `v`, then movement and `y` / `d` / `x` / `p` | Select text, then copy / cut / replace |
| `Enter` in insert mode or on a dialog button | Submit a word or apply a dialog action |
| `Enter` on a history entry | Focus its details |
| `/`, `Ctrl-r` | History filters, refresh |
| `yy` | Copy the highlighted translation from the details pane |
| `ya` / `yq` | Copy all translation values / the selected query |
| `q` in navigation mode, or `Ctrl-C` | Exit |

Copying writes plain text to the local desktop clipboard; multiple values are separated by newlines. If the clipboard is unavailable, voci explains the failure. Remote-terminal clipboard protocols are not supported. On Linux, clipboard content is owned while the TUI runs; retaining it after exit depends on the desktop clipboard manager. Failed and unfinished attempts allow copying the query, but have no translations to copy.

Unicode grapheme editing and single-line paste remain supported. The terminal needs at least 24 columns and 12 rows; resizing preserves state. Terminal mode, cursor, and paste settings are restored on exit and recoverable failures. Normal shutdown allows two seconds to finish recording cancellations.

### Keybinding profiles

The first TUI launch installs missing presets in a `keybindings/` folder beside the effective `config.toml`, preserving existing files. A missing default config file is not created or rewritten. With no profile selected, voci uses built-in QWERTY defaults.

```text
<config directory>/
├── config.toml
└── keybindings/
    ├── qwerty.keybinding.toml
    └── neo-noted.keybinding.toml
```

Select a profile in your config:

```toml
[tui]
keybindings = "keybindings/neo-noted.keybinding.toml"
```

Relative paths are resolved against the config file's directory, including `--config PATH`; absolute paths also work. The predefined Neo Noted navigation is:

```toml
[navigation]
left = ["Left", "t"]
right = ["Right", "r"]
up = ["Up", "m"]
down = ["Down", "n"]
home = ["Home", "gg", "b"]
end = ["End", "G", "l"]
```

Each array lists alternative bindings. `gg` is two successive lowercase presses; `G` is uppercase. Letters come from your active layout, not QWERTY key positions. An explicit array replaces that action's defaults; omitted actions retain defaults. Keep named keys in the array when you want arrows or Home/End alongside letter shortcuts. Empty arrays, unknown actions, and conflicting complete/prefix bindings are rejected. Shared prefixes such as `gg` and `gt` are supported with a 750 ms inter-key timeout.

`[navigation]` also accepts `page_up` and `page_down`. `[actions]` accepts `next_tab`, `previous_tab`, `pane_prefix`, `edit`, `append`, `undo`, `redo`, `paste`, `word_begin`, `word_end`, `visual`, `yank_selection`, `delete_selection`, `change_selection`, `submit`, `next_focus`, `previous_focus`, `filter`, `copy_value`, `copy_all`, `copy_query`, `refresh`, `quit`, and `cancel`. For example:

```toml
[actions]
copy_query = ["yq", "Alt-q"]
word_begin = ["Ctrl-Left", "b"]
word_end = ["Ctrl-Right", "e"]
delete_selection = ["d", "x"]
```

Named keys include `Left`, `Right`, `Up`, `Down`, `Home`, `End`, `PageUp`, `PageDown`, `Enter`, `Tab`, `Esc`, and `Space`, with `Ctrl-`, `Alt-`, or `Shift-` modifiers. Space-separated tokens can express modified sequences; compact character sequences such as `yy` also work. Input shortcuts are active in normal/visual modes; insert mode retains ordinary typing and cursor editing. Word-motion bindings take precedence over navigation aliases inside input fields; conflicting input actions are still rejected. Modified word bindings also work in insert mode. Existing profile files are preserved; add `x` to an explicit `delete_selection = ["d"]` override to enable both cut keys. `pane_prefix` names the pane-mode toggle for compatibility with existing profiles. Copy-selection bindings are resolved in visual mode, separately from result-copy sequences such as `yy`. Ctrl-C remains reserved for emergency exit. Profiles reload on the next launch; there is no profile inheritance or live editor.

## CLI errors

Successful results go to stdout; failures go to stderr without raw provider responses or stack traces. Exit statuses are `0` for success/help, `2` for invalid invocation/input/languages, `1` for unsuccessful lookup or configuration/provider failure, and `130` for an interrupted CLI lookup. Missing entries, ambiguous sources, unsupported languages, authentication/quota errors, network failures, and timeouts have different recovery messages.

## Development and verification

```sh
cargo fmt --all -- --check
cargo clippy --locked --all-targets -- -D warnings
cargo test --locked
```

Normal tests use synthetic SQLite dictionaries and local mock HTTP servers; they need no credentials, dictionary downloads, or external network. The live feasibility test is ignored by default and consumes quota when explicitly run:

```sh
# Set VOCI_MICROSOFT_KEY and, when needed, VOCI_MICROSOFT_REGION first.
cargo test --locked --test live_microsoft -- --ignored --nocapture
```

Review the printed candidates and latency, especially multiple meanings for `Verbindlichkeit` and `liability`, ambiguous `Gift`, inflections, and misses. Successful automated assertions alone do not establish dictionary quality. Record findings in [provider validation](docs/pitches/lookup/provider-validation.md).

The application service owns validation and language resolution. The provider adapters own WikDict files/SQL and Microsoft HTTP payloads. The shared coordinator records append-only history events around provider setup and lookup. CLI/TUI presentation uses structured results and safe terminal text. History tests use isolated temporary application-data directories; no provider registry or machine-translation fallback is introduced.

Lookup and filter fields share an [edtui](https://github.com/preiter93/edtui) editor through `src/tui/input.rs`. edtui owns the text buffer, modal state, selections, and editing actions. The adapter maps voci's configurable bindings to these actions, preserves paste-at-cursor behavior, and commits cuts only after clipboard copying succeeds. Since edtui 0.11 uses Unicode scalar positions, the adapter keeps grapheme-aware motions and terminal rendering for combining accents and emoji. Its default key handler is not enabled: the configured voci actions remain the supported shortcuts. Optional edtui syntax highlighting, mouse handling, and system clipboard features are disabled; voci retains its shared clipboard service. The adapter groups edtui snapshots into user edits and tracks available redo steps because edtui 0.11 does not invalidate its redo stack on a new edit.
