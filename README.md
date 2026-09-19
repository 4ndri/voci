# voci

Quick German ↔ English dictionary lookup in your terminal. WikDict is the default: no account or API key required.

```sh
voci Verbindlichkeit
voci liability
voci --from de --to en Verbindlichkeit
voci shell
```

The CLI prints a word, its language direction, and up to eight translation candidates. The TUI keeps the same lookup loop open for repeated searches. Results remain in memory only; history and vocabulary learning are future features.

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

The first lookup automatically downloads the two German/English databases from [WikDict](https://www.wikdict.com/page/download), about 45 MiB total, and builds a local lookup index. Download progress goes to stderr; Ctrl-C cancels setup. Later lookups use the installed files without contacting any service. Starting `voci shell` prepares missing dictionaries before opening the TUI.

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

## TUI

`voci shell` requires an interactive terminal on both stdin and stdout. Start with optional `--from`, `--to`, `--provider`, and `--config` flags. It shows one input, language selectors, and the current result or actionable error. Selections last for the session and are not written to disk.

| Key | Action |
| --- | --- |
| Enter | Submit the current word or retry; replace an active request |
| Tab / Shift-Tab | Move between word, source, target, and results |
| Left / Right, Home / End | Edit the input cursor; arrow keys change a focused language selector |
| Backspace / Delete | Remove a Unicode grapheme |
| Up / Down, Page Up / Page Down | Scroll focused results |
| Escape | Cancel a pending request; otherwise exit |
| Ctrl-C | Exit immediately |

Paste inserts a single query into the word field. Multiline/control-character paste is rejected. `q` is an ordinary letter. The terminal needs at least 24 columns and 12 rows for the full view; resizing is supported. Raw mode, alternate screen, cursor, and paste mode are restored on normal exit and recoverable failures, including panic unwinding.

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

The application service owns validation and language resolution. The provider adapters own WikDict files/SQL and Microsoft HTTP payloads. CLI/TUI presentation uses structured results, preserving provider identity, result kind, and all candidates for future history integration. Only dictionary files are persisted; there is no lookup history, provider registry, or machine-translation fallback.
