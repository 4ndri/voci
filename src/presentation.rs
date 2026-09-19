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

pub fn render_history(entry: &crate::history::HistoryEntry) -> String {
    let mut lines = vec![
        format!(
            "{} · {} · {}",
            safe_text(&entry.query),
            crate::history::display_time(entry.started_at),
            entry.status()
        ),
        format!(
            "{} → {} · {}",
            entry.from.map_or("?", |l| l.code()),
            entry.to.map_or("?", |l| l.code()),
            safe_text(entry.provider.as_deref().unwrap_or("unknown provider"))
        ),
    ];
    if let Some(time) = entry.finished_at {
        lines.push(format!("Finished {}", crate::history::display_time(time)));
    }
    if let Some(result) = entry.result() {
        for (i, c) in result.candidates.iter().enumerate() {
            let value = crate::clipboard::values(result, Some(i)).unwrap_or_default();
            let sense = c
                .sense
                .as_ref()
                .map(|s| format!(" · {}", safe_text(s)))
                .unwrap_or_default();
            lines.push(format!("{}. {value}{sense}", i + 1));
        }
        if let Some(attribution) = &result.attribution {
            lines.push(safe_text(attribution));
        }
    } else if let Some(message) = entry.finished.as_ref().and_then(|f| f.message.as_ref()) {
        lines.push(safe_text(message));
    }
    format!("{}\n", lines.join("\n"))
}
