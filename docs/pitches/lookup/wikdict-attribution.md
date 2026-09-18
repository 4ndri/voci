# WikDict dictionary data

voci downloads the `2_2026-06` release of WikDict's German/English translation databases:

- [German → English database](https://download.wikdict.com/dictionaries/sqlite/2_2026-06/de-en.sqlite3)
- [English → German database](https://download.wikdict.com/dictionaries/sqlite/2_2026-06/en-de.sqlite3)

WikDict is by [Karl Bartel](https://www.karl.berlin/). Its data is extracted from [Wiktionary](https://www.wiktionary.org/) contributors' work via [DBnary](https://kaiko.getalp.org/about-dbnary/). WikDict publishes the data under the [Creative Commons Attribution-ShareAlike 4.0 International license](https://creativecommons.org/licenses/by-sa/4.0/) ([legal code](https://creativecommons.org/licenses/by-sa/4.0/legalcode.en)), as linked on its [download page](https://www.wikdict.com/page/download).

voci preserves the downloaded translation records and adds an index for case-insensitive, canonically normalized Unicode lookup. Display transformations split translation lists into candidates, remove identical duplicates, preserve sense distinctions, and show a concise subset. These changes are made by voci, not by the original contributors. The dictionary data and adaptations remain under CC BY-SA 4.0; this notice does not apply that data license to unrelated application code.

Every successful WikDict lookup includes provider/contributor credit, the license name, and a source link. When redistributing dictionary files or adapted dictionary content, retain source attribution and the applicable license notices. No endorsement by WikDict, Wiktionary, or DBnary is implied.
