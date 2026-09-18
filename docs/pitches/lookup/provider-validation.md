# Dictionary provider validation

Microsoft Dictionary Lookup remains an optional provider. Its documented API supports multiple translations and German ↔ English dictionary lookup. Direct German ↔ French dictionary support is not provided by this adapter. [API documentation](https://learn.microsoft.com/en-us/azure/ai-services/translator/text-translation/reference/v3/dictionary-lookup)

## WikDict validation

The default provider uses WikDict release `2_2026-06`. A real CLI run downloaded both official databases without credentials, validated/indexed them, and looked up `Verbindlichkeit`. It returned `courtesy`, `liability`, and `obligation` in upstream ranking order. This is the observed data, not the pitch's illustrative list; results should not be mistaken for a definitive translation.

Subsequent CLI checks used a deliberately unreachable HTTPS proxy to verify that the installed dictionaries work offline. `liability` returned several German candidates, `Gift` was ambiguous, `--from de Gift` returned poison-related and other dictionary meanings, and `GRÜßE` matched `Grüße`. A fabricated word returned an undetermined-source error. Warm commands took approximately 4 ms in this Linux workspace; this is an observation, not a cross-platform performance guarantee.

The adapter uses only the two translation databases (about 45 MiB downloaded), not the monolingual inflection datasets. `Verbindlichkeiten` was absent in the inspected translation tables; callers may need the base form. Lookup preserves sense distinctions and the provider's ranking. See [attribution](wikdict-attribution.md).

## Microsoft live feasibility status

**Pending:** no `VOCI_MICROSOFT_KEY` was available during implementation. No authenticated lookup has been performed, and neither real dictionary quality nor real network latency has been verified. Synthetic fixtures are used only for deterministic tests; their translations are not evidence of Microsoft's actual response.

With credentials configured, run:

```sh
cargo test --locked --test live_microsoft -- --ignored --nocapture
```

The test exercises `Verbindlichkeit`, `liability`, `Gift`, `Verbindlichkeiten`, `running`, and a fabricated missing word, in both directions. It prints response latency and candidates. Verify that the main examples give useful alternatives, automatic source inference is practical, inflections behave acceptably, and waiting does not undermine the lookup loop.

Before treating the provider as validated, record the date, observed candidate quality and latency, authentication/region setup, quota/cost suitability, and applicable usage/attribution terms. Confirm separately whether future storage of returned translations is permitted before introducing history. Do not record keys or other credentials here.

If the source is unsuitable, revisit the provider decision. Do not substitute machine translation or scraping silently.

## Platform validation

Local Linux checks passed: `cargo fmt --all -- --check`, `cargo clippy --locked --all-targets -- -D warnings`, and 33 automated tests. The Microsoft live feasibility test remains ignored. Tests cover mock HTTP behavior, language resolution, concurrent requests and deadline cancellation, CLI/configuration errors, Unicode editing, result rendering, stale TUI responses, WikDict download validation/recovery, concurrent installation, and keyless provider selection.

A Linux pseudo-terminal smoke test passed Unicode paste, source/target selection, shrinking and restoring the window, cancellation/retry, Ctrl-C exit, and Escape exit. A local stalled proxy kept requests away from Microsoft. The test verified restored terminal attributes, alternate screen, cursor visibility, and bracketed paste mode after both exits. This is terminal validation, not live dictionary validation.

A subsequent Linux pseudo-terminal test used the real downloaded WikDict files with no API key and an unreachable HTTPS proxy. It rendered translations and attribution for `Verbindlichkeit` and restored terminal attributes on exit.

The CI workflow defines formatting, Clippy, and tests on Linux, macOS, and Windows. Adding the workflow does not establish that remote CI has run. Interactive terminal smoke checks on macOS and Windows remain pending in this Linux workspace.

On each platform, verify Unicode input and paste, language selection, resize, loading/cancellation, retry, and exit. Confirm the caller's terminal echoes normally after Escape, Ctrl-C, and an error. For a real provider lookup, also verify a successful result and an actionable authentication/network failure.
