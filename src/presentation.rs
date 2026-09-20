use crate::domain::LookupResult;
use unicode_segmentation::UnicodeSegmentation;

pub const CANDIDATE_LIMIT: usize = 8;

/// Prevent escape sequences, line injection, and bidi overrides in terminal output.
pub fn safe_text(text: &str) -> String {
    text.chars().filter(|c| !c.is_control() && !matches!(*c, '\u{061c}' | '\u{200e}' | '\u{200f}' | '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}')).collect()
}

pub fn result_lines(result: &LookupResult) -> Vec<String> {
    let mut lines = vec![
        format!("{} · {}", safe_text(&result.headword), result.pair),
        String::new(),
    ];
    for (index, candidate) in result.candidates.iter().take(CANDIDATE_LIMIT).enumerate() {
        let prefix = if candidate.prefix.is_empty() {
            String::new()
        } else {
            format!("{} ", safe_text(&candidate.prefix))
        };
        let duplicate_text = result
            .candidates
            .iter()
            .filter(|other| other.normalized == candidate.normalized)
            .count()
            > 1;
        let label = if duplicate_text {
            let mut labels = Vec::new();
            if let Some(pos) = &candidate.part_of_speech {
                labels.push(safe_text(&pos.to_lowercase()));
            }
            if let Some(meaning) = candidate
                .sense
                .as_ref()
                .or(candidate.back_translations.first())
            {
                let meaning = safe_text(meaning);
                let mut short: String = meaning.graphemes(true).take(72).collect();
                if short.len() < meaning.len() {
                    short.push('…');
                }
                labels.push(short);
            }
            if labels.is_empty() {
                String::new()
            } else {
                format!(" ({})", labels.join("; "))
            }
        } else {
            String::new()
        };
        lines.push(format!(
            "{}. {}{}{}",
            index + 1,
            prefix,
            safe_text(&candidate.text),
            label
        ));
    }
    if result.candidates.len() > CANDIDATE_LIMIT {
        lines.push(format!(
            "… {} more candidates omitted.",
            result.candidates.len() - CANDIDATE_LIMIT
        ));
    }
    if let Some(attribution) = &result.attribution {
        lines.push(safe_text(attribution));
    }
    lines
}

pub fn render_result(result: &LookupResult) -> String {
    format!("{}\n", result_lines(result).join("\n"))
}

/// Render a saved encounter at the default redirected-output width.
pub fn render_history(entry: &crate::history::HistoryEntry) -> String {
    render_history_at_width(entry, 100)
}

pub fn render_history_at_width(entry: &crate::history::HistoryEntry, width: usize) -> String {
    let width = width.clamp(16, 120);
    let content_width = width - 4;
    let mut lines = vec![border(&[content_width], '┌', '┬', '┐')];
    lines.extend(table_row(
        &[format!("Request: {}", entry.query)],
        &[content_width],
    ));
    lines.push(border(&[content_width], '├', '┼', '┤'));
    let mut metadata = format!(
        "{} · {} → {} · {} · {}",
        crate::history::display_time(entry.started_at),
        entry.from.map_or("?", |l| l.code()),
        entry.to.map_or("?", |l| l.code()),
        entry.provider.as_deref().unwrap_or("unknown provider"),
        entry.status(),
    );
    if let Some(time) = entry.finished_at {
        metadata.push_str(&format!(
            " · Finished {}",
            crate::history::display_time(time)
        ));
    }
    lines.extend(table_row(&[metadata], &[content_width]));
    lines.push(border(&[content_width], '├', '┼', '┤'));
    if let Some(result) = entry.result() {
        // A compact single-column sub-table keeps all fields accessible in narrow terminals.
        let widths = if width < 60 {
            vec![content_width - 4]
        } else {
            let number = result.candidates.len().to_string().len().max(1);
            let remaining = content_width - 10 - number;
            vec![number, remaining * 2 / 5, remaining - remaining * 2 / 5]
        };
        let headers = if widths.len() == 1 {
            vec!["Lookup results".into()]
        } else {
            vec![
                "#".into(),
                "Translation".into(),
                "Meaning / part of speech".into(),
            ]
        };
        let mut nested = vec![border(&widths, '┌', '┬', '┐')];
        nested.extend(table_row(&headers, &widths));
        nested.push(border(&widths, '├', '┼', '┤'));
        for (i, candidate) in result.candidates.iter().enumerate() {
            if i > 0 {
                nested.push(border(&widths, '├', '┼', '┤'));
            }
            let translation = crate::clipboard::values(result, Some(i)).unwrap_or_default();
            let meaning = candidate
                .sense
                .as_ref()
                .or(candidate.back_translations.first());
            let details = [
                candidate.part_of_speech.as_deref(),
                meaning.map(String::as_str),
            ]
            .into_iter()
            .flatten()
            .collect::<Vec<_>>()
            .join(" · ");
            let cells = if widths.len() == 1 {
                vec![format!(
                    "{}. {}{}",
                    i + 1,
                    translation,
                    if details.is_empty() {
                        String::new()
                    } else {
                        format!(" · {details}")
                    }
                )]
            } else {
                vec![(i + 1).to_string(), translation, details]
            };
            nested.extend(table_row(&cells, &widths));
        }
        nested.push(border(&widths, '└', '┴', '┘'));
        for line in nested {
            lines.push(format!("│ {line} │"));
        }
        if let Some(attribution) = &result.attribution {
            lines.push(border(&[content_width], '├', '┼', '┤'));
            lines.extend(table_row(
                &[format!("Source: {attribution}")],
                &[content_width],
            ));
        }
    } else {
        let message = entry
            .finished
            .as_ref()
            .and_then(|f| f.message.as_deref())
            .unwrap_or("No lookup result recorded.");
        lines.extend(table_row(&[message.into()], &[content_width]));
    }
    lines.push(border(&[content_width], '└', '┴', '┘'));
    format!("{}\n", lines.join("\n"))
}

fn border(widths: &[usize], left: char, middle: char, right: char) -> String {
    format!(
        "{left}{}{right}",
        widths
            .iter()
            .map(|w| "─".repeat(w + 2))
            .collect::<Vec<_>>()
            .join(&middle.to_string())
    )
}

fn table_row(cells: &[String], widths: &[usize]) -> Vec<String> {
    use unicode_width::UnicodeWidthStr;
    let wrapped: Vec<_> = cells
        .iter()
        .zip(widths)
        .map(|(cell, &width)| wrap_cell(cell, width))
        .collect();
    (0..wrapped.iter().map(Vec::len).max().unwrap_or(1))
        .map(|row| {
            let cells = wrapped
                .iter()
                .zip(widths)
                .map(|(lines, &width)| {
                    let text = lines.get(row).map_or("", String::as_str);
                    format!(" {text}{} ", " ".repeat(width.saturating_sub(text.width())))
                })
                .collect::<Vec<_>>();
            format!("│{}│", cells.join("│"))
        })
        .collect()
}

// Wrap at words where possible and split long words only between graphemes.
fn wrap_cell(text: &str, width: usize) -> Vec<String> {
    use unicode_width::UnicodeWidthStr;
    let text = safe_text(text);
    let mut lines = Vec::new();
    let mut line = String::new();
    for word in text.split_whitespace() {
        if !line.is_empty() {
            if line.width() + 1 + word.width() <= width {
                line.push(' ');
            } else {
                lines.push(std::mem::take(&mut line));
            }
        }
        for grapheme in word.graphemes(true) {
            if !line.is_empty() && line.width() + grapheme.width() > width {
                lines.push(std::mem::take(&mut line));
            }
            line.push_str(grapheme);
        }
    }
    if !line.is_empty() || lines.is_empty() {
        lines.push(line);
    }
    lines
}
