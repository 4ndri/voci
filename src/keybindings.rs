use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use serde::Deserialize;
use std::{
    collections::BTreeMap,
    path::Path,
    time::{Duration, Instant},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    Left,
    Right,
    Up,
    Down,
    Home,
    End,
    PageUp,
    PageDown,
    NextTab,
    PreviousTab,
    Pane,
    Edit,
    Append,
    Undo,
    Redo,
    Paste,
    WordBegin,
    WordEnd,
    Visual,
    YankSelection,
    DeleteSelection,
    ChangeSelection,
    Submit,
    NextFocus,
    PreviousFocus,
    Filter,
    CopyValue,
    CopyAll,
    CopyQuery,
    Refresh,
    Quit,
    Cancel,
}
#[derive(Clone, Copy)]
pub enum Context {
    Lookup = 1,
    History = 2,
    Input = 4,
    Visual = 8,
    Pane = 16,
    Insert = 32,
    Dialog = 64,
}
impl Action {
    fn contexts(self) -> u8 {
        match self {
            Self::Left | Self::Right | Self::Up | Self::Down | Self::Pane | Self::Cancel => 31,
            Self::Filter | Self::Refresh => 2,
            Self::Edit => 1 | 4 | 8,
            Self::Append | Self::Undo | Self::Redo => 4,
            Self::Paste
            | Self::Visual
            | Self::WordBegin
            | Self::WordEnd
            | Self::DeleteSelection => 4 | 8,
            Self::YankSelection | Self::ChangeSelection => 8,
            Self::CopyValue | Self::CopyAll | Self::CopyQuery => 1 | 2 | 4,
            _ => 1 | 2 | 4 | 8,
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Key {
    code: KeyCode,
    modifiers: KeyModifiers,
}
impl Key {
    fn command(self) -> bool {
        !matches!(self.code, KeyCode::Char(_))
            || self
                .modifiers
                .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT)
    }
    fn event(event: KeyEvent) -> Self {
        let mut modifiers = event.modifiers;
        if matches!(event.code, KeyCode::Char(_)) {
            modifiers.remove(KeyModifiers::SHIFT);
        }
        let code = if event.code == KeyCode::BackTab {
            modifiers.insert(KeyModifiers::SHIFT);
            KeyCode::Tab
        } else {
            event.code
        };
        Self { code, modifiers }
    }
}
#[derive(Clone, Debug)]
struct Binding {
    action: Action,
    keys: Vec<Key>,
    label: String,
    contexts: u8,
}
#[derive(Clone, Debug)]
pub struct Keybindings {
    bindings: Vec<Binding>,
}
#[derive(Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct Profile {
    navigation: BTreeMap<String, Vec<String>>,
    actions: BTreeMap<String, Vec<String>>,
}
const QWERTY: &str = include_str!("../assets/keybindings/qwerty.keybinding.toml");
const NEO: &str = include_str!("../assets/keybindings/neo-noted.keybinding.toml");
fn defaults() -> Vec<(&'static str, Action, Vec<&'static str>)> {
    vec![
        ("left", Action::Left, vec!["Left", "h"]),
        ("right", Action::Right, vec!["Right", "l"]),
        ("up", Action::Up, vec!["Up", "k"]),
        ("down", Action::Down, vec!["Down", "j"]),
        ("home", Action::Home, vec!["Home", "gg"]),
        ("end", Action::End, vec!["End", "G"]),
        ("page_up", Action::PageUp, vec!["PageUp"]),
        ("page_down", Action::PageDown, vec!["PageDown"]),
        ("next_tab", Action::NextTab, vec!["gt"]),
        ("previous_tab", Action::PreviousTab, vec!["gT"]),
        ("pane_prefix", Action::Pane, vec!["Ctrl-w"]),
        ("edit", Action::Edit, vec!["i"]),
        ("append", Action::Append, vec!["a"]),
        ("undo", Action::Undo, vec!["u"]),
        ("redo", Action::Redo, vec!["Ctrl-r"]),
        ("paste", Action::Paste, vec!["p"]),
        ("word_begin", Action::WordBegin, vec!["Ctrl-Left", "b"]),
        ("word_end", Action::WordEnd, vec!["Ctrl-Right", "e"]),
        ("visual", Action::Visual, vec!["v"]),
        ("yank_selection", Action::YankSelection, vec!["y"]),
        ("delete_selection", Action::DeleteSelection, vec!["d", "x"]),
        ("change_selection", Action::ChangeSelection, vec!["c"]),
        ("submit", Action::Submit, vec!["Enter"]),
        ("next_focus", Action::NextFocus, vec!["Tab"]),
        ("previous_focus", Action::PreviousFocus, vec!["Shift-Tab"]),
        ("filter", Action::Filter, vec!["/"]),
        ("copy_value", Action::CopyValue, vec!["yy"]),
        ("copy_all", Action::CopyAll, vec!["ya"]),
        ("copy_query", Action::CopyQuery, vec!["yq"]),
        ("refresh", Action::Refresh, vec!["Ctrl-r"]),
        ("quit", Action::Quit, vec!["q"]),
        ("cancel", Action::Cancel, vec!["Esc"]),
    ]
}
impl Default for Keybindings {
    fn default() -> Self {
        Self::parse("").expect("built-in bindings are valid")
    }
}
impl Keybindings {
    pub fn parse(text: &str) -> Result<Self, String> {
        let mut profile: Profile = toml::from_str(text).map_err(|_| {
            "Invalid keybinding TOML; expected [navigation] and [actions] arrays.".to_string()
        })?;
        let mut bindings = Vec::new();
        for (index, (name, action, default)) in defaults().into_iter().enumerate() {
            let section = if index < 8 {
                &mut profile.navigation
            } else {
                &mut profile.actions
            };
            let labels = section
                .remove(name)
                .unwrap_or_else(|| default.into_iter().map(str::to_string).collect());
            if labels.is_empty() {
                return Err(format!("Binding {name} must have at least one key."));
            }
            for label in labels {
                let keys = parse_sequence(&label)?;
                if keys.iter().any(|k| {
                    matches!(k.code, KeyCode::Char('c' | 'C'))
                        && k.modifiers.contains(KeyModifiers::CONTROL)
                }) {
                    return Err("Ctrl-c is reserved for emergency exit.".into());
                }
                let mut contexts = action.contexts();
                if matches!(
                    action,
                    Action::Submit
                        | Action::NextFocus
                        | Action::PreviousFocus
                        | Action::Pane
                        | Action::Cancel
                        | Action::WordBegin
                        | Action::WordEnd
                ) && keys.first().is_some_and(|key| key.command())
                {
                    contexts |= Context::Insert as u8;
                }
                if matches!(
                    action,
                    Action::Submit
                        | Action::NextFocus
                        | Action::PreviousFocus
                        | Action::Pane
                        | Action::Cancel
                        | Action::Left
                        | Action::Right
                ) {
                    contexts |= Context::Dialog as u8;
                }
                bindings.push(Binding {
                    action,
                    keys,
                    label,
                    contexts,
                });
            }
        }
        if !profile.navigation.is_empty() || !profile.actions.is_empty() {
            return Err(
                "Unknown keybinding action; check [navigation] and [actions] names.".into(),
            );
        }
        // Input word motions take precedence over navigation aliases only in inputs.
        // In particular, Neo Noted's `b` still means Home in lists, but word-begin in text.
        let word_keys = bindings
            .iter()
            .filter(|b| matches!(b.action, Action::WordBegin | Action::WordEnd))
            .map(|b| b.keys.clone())
            .collect::<Vec<_>>();
        for binding in &mut bindings {
            if matches!(
                binding.action,
                Action::Left
                    | Action::Right
                    | Action::Up
                    | Action::Down
                    | Action::Home
                    | Action::End
                    | Action::PageUp
                    | Action::PageDown
            ) && word_keys
                .iter()
                .any(|keys| keys.starts_with(&binding.keys) || binding.keys.starts_with(keys))
            {
                binding.contexts &= !(Context::Input as u8 | Context::Visual as u8);
            }
        }
        for (i, a) in bindings.iter().enumerate() {
            for b in &bindings[i + 1..] {
                if a.contexts & b.contexts != 0
                    && (a.keys.starts_with(&b.keys) || b.keys.starts_with(&a.keys))
                {
                    return Err(format!(
                        "Conflicting bindings '{}' and '{}'.",
                        a.label, b.label
                    ));
                }
            }
        }
        Ok(Self { bindings })
    }
    /// Write only absent presets, never rewrite a user's config or customized profile.
    pub fn load(
        config_path: &Path,
        selected: Option<&Path>,
    ) -> Result<(Self, Vec<String>), String> {
        let parent = config_path.parent().unwrap_or_else(|| Path::new("."));
        let directory = parent.join("keybindings");
        let mut warnings = vec![];
        for (name, body) in [
            ("qwerty.keybinding.toml", QWERTY),
            ("neo-noted.keybinding.toml", NEO),
        ] {
            let result = (|| -> std::io::Result<()> {
                use std::io::Write;
                std::fs::create_dir_all(&directory)?;
                // Link a completely written temporary file, without replacing an existing profile.
                let mut file = tempfile::NamedTempFile::new_in(&directory)?;
                file.write_all(body.as_bytes())?;
                file.as_file().sync_all()?;
                match file.persist_noclobber(directory.join(name)) {
                    Ok(_) => Ok(()),
                    Err(e) if e.error.kind() == std::io::ErrorKind::AlreadyExists => Ok(()),
                    Err(e) => Err(e.error),
                }
            })();
            if result.is_err() {
                warnings.push(format!(
                    "Cannot install keybinding preset {}.",
                    directory.join(name).display()
                ));
            }
        }
        let Some(selected) = selected else {
            return Ok((Self::default(), warnings));
        };
        let path = if selected.is_absolute() {
            selected.to_owned()
        } else {
            parent.join(selected)
        };
        let text = std::fs::read_to_string(&path)
            .map_err(|_| format!("Cannot read keybinding profile {}.", path.display()))?;
        let bindings = Self::parse(&text)
            .map_err(|e| format!("Keybinding profile {}: {e}", path.display()))?;
        Ok((bindings, warnings))
    }
    pub fn label(&self, action: Action) -> String {
        self.bindings
            .iter()
            .filter(|b| b.action == action)
            .map(|b| b.label.as_str())
            .collect::<Vec<_>>()
            .join("/")
    }
}
fn parse_sequence(value: &str) -> Result<Vec<Key>, String> {
    fn named(token: &str) -> Option<Key> {
        let (modifiers, name) = if let Some(v) = token.strip_prefix("Ctrl-") {
            (KeyModifiers::CONTROL, v)
        } else if let Some(v) = token.strip_prefix("Shift-") {
            (KeyModifiers::SHIFT, v)
        } else if let Some(v) = token.strip_prefix("Alt-") {
            (KeyModifiers::ALT, v)
        } else {
            (KeyModifiers::NONE, token)
        };
        let code = match name {
            "Left" => KeyCode::Left,
            "Right" => KeyCode::Right,
            "Up" => KeyCode::Up,
            "Down" => KeyCode::Down,
            "Home" => KeyCode::Home,
            "End" => KeyCode::End,
            "PageUp" => KeyCode::PageUp,
            "PageDown" => KeyCode::PageDown,
            "Enter" => KeyCode::Enter,
            "Tab" => KeyCode::Tab,
            "Esc" | "Escape" => KeyCode::Esc,
            "Space" => KeyCode::Char(' '),
            _ if name.chars().count() == 1 => KeyCode::Char(name.chars().next().unwrap()),
            _ => return None,
        };
        let code = match code {
            KeyCode::Char(c) if modifiers.contains(KeyModifiers::SHIFT) => {
                KeyCode::Char(c.to_ascii_uppercase())
            }
            other => other,
        };
        Some(Key::event(KeyEvent::new(code, modifiers)))
    }
    if value.is_empty() || value.chars().any(char::is_control) {
        return Err("Keybinding cannot be empty or contain control characters.".into());
    }
    if let Some(key) = named(value) {
        return Ok(vec![key]);
    }
    if value.contains('-') || value.contains(' ') {
        return value
            .split_whitespace()
            .map(|part| named(part).ok_or_else(|| format!("Unknown key token '{part}'.")))
            .collect();
    }
    if value.chars().count() > 8 {
        return Err("Key sequences are limited to eight characters.".into());
    }
    Ok(value
        .chars()
        .map(|c| Key {
            code: KeyCode::Char(c),
            modifiers: KeyModifiers::NONE,
        })
        .collect())
}
#[derive(Default)]
pub struct Resolver {
    pending: Vec<Key>,
    last: Option<Instant>,
    context: u8,
}
impl Resolver {
    pub fn reset(&mut self) {
        self.pending.clear();
        self.last = None;
    }
    pub fn pending(&self) -> bool {
        !self.pending.is_empty()
            && self
                .last
                .is_some_and(|last| last.elapsed() < Duration::from_millis(750))
    }
    pub fn feed(
        &mut self,
        bindings: &Keybindings,
        event: KeyEvent,
        context: Context,
        now: Instant,
    ) -> Option<Action> {
        let context = context as u8;
        if self.context != context
            || self
                .last
                .is_some_and(|last| now.duration_since(last) >= Duration::from_millis(750))
        {
            self.reset();
        }
        self.context = context;
        self.last = Some(now);
        self.pending.push(Key::event(event));
        let candidates = |keys: &[Key]| {
            bindings
                .bindings
                .iter()
                .filter(|b| b.contexts & context != 0 && b.keys.starts_with(keys))
                .collect::<Vec<_>>()
        };
        let mut matches = candidates(&self.pending);
        if matches.is_empty() {
            self.pending = vec![Key::event(event)];
            matches = candidates(&self.pending);
        }
        if let Some(binding) = matches.iter().find(|b| b.keys == self.pending) {
            let action = binding.action;
            self.pending.clear();
            return Some(action);
        }
        if matches.is_empty() {
            self.reset();
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn key(c: char) -> KeyEvent {
        KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE)
    }
    #[test]
    fn neo_aliases_sequences_and_contexts() {
        let b = Keybindings::parse(NEO).unwrap();
        let mut r = Resolver::default();
        let now = Instant::now();
        assert_eq!(r.feed(&b, key('g'), Context::History, now), None);
        assert_eq!(
            r.feed(&b, key('g'), Context::History, now),
            Some(Action::Home)
        );
        assert_eq!(
            r.feed(&b, key('b'), Context::History, now),
            Some(Action::Home)
        );
        assert_eq!(
            r.feed(&b, key('l'), Context::History, now),
            Some(Action::End)
        );
        assert_eq!(
            r.feed(&b, key('G'), Context::History, now),
            Some(Action::End)
        );
        assert_eq!(r.feed(&b, key('h'), Context::History, now), None);
        assert_eq!(
            r.feed(
                &b,
                KeyEvent::new(KeyCode::Char('w'), KeyModifiers::CONTROL),
                Context::History,
                now
            ),
            Some(Action::Pane)
        );
        assert_eq!(r.feed(&b, key('t'), Context::Pane, now), Some(Action::Left));
        r.feed(&b, key('g'), Context::History, now);
        assert_eq!(
            r.feed(&b, key('t'), Context::History, now + Duration::from_secs(1)),
            Some(Action::Left)
        );
        assert!(Keybindings::parse("[navigation]\nhome=['g']").is_err());
        assert!(Keybindings::parse("[navigation]\nleft=['i']").is_err());
        assert!(Keybindings::parse("[navigation]\nleft=[]").is_err());
    }
    #[test]
    fn insert_sequences_timeout_and_do_not_start_with_ordinary_letters() {
        let bindings = Keybindings::parse("[actions]\nsubmit=['Ctrl-x s','zz']").unwrap();
        let mut resolver = Resolver::default();
        let now = Instant::now();
        let prefix = KeyEvent::new(KeyCode::Char('x'), KeyModifiers::CONTROL);
        assert_eq!(
            resolver.feed(&bindings, key('z'), Context::Insert, now),
            None
        );
        assert!(!resolver.pending());
        assert_eq!(resolver.feed(&bindings, prefix, Context::Insert, now), None);
        assert_eq!(
            resolver.feed(&bindings, key('s'), Context::Insert, now),
            Some(Action::Submit)
        );
        resolver.feed(&bindings, prefix, Context::Insert, now);
        assert_eq!(
            resolver.feed(
                &bindings,
                key('s'),
                Context::Insert,
                now + Duration::from_secs(1)
            ),
            None
        );
        assert!(!resolver.pending());
        resolver.feed(
            &bindings,
            prefix,
            Context::Insert,
            now + Duration::from_secs(2),
        );
        assert_eq!(
            resolver.feed(
                &bindings,
                key('s'),
                Context::Dialog,
                now + Duration::from_secs(2)
            ),
            None
        );
    }
    #[test]
    fn presets_preserve_edits_and_resolve_relative_to_config() {
        let dir = tempfile::tempdir().unwrap();
        let config = dir.path().join("config.toml");
        Keybindings::load(&config, None).unwrap();
        let profile = dir.path().join("keybindings/neo-noted.keybinding.toml");
        std::fs::write(&profile, "[navigation]\nhome=['Home','b']").unwrap();
        let (b, _) = Keybindings::load(
            &config,
            Some(Path::new("keybindings/neo-noted.keybinding.toml")),
        )
        .unwrap();
        assert_eq!(b.label(Action::Home), "Home/b");
        assert!(std::fs::read_to_string(profile).unwrap().contains("'b'"));
    }
    #[test]
    fn input_word_motions_shadow_navigation_only_in_text_contexts() {
        let bindings = Keybindings::parse(NEO).unwrap();
        let mut resolver = Resolver::default();
        let now = Instant::now();
        assert_eq!(
            resolver.feed(&bindings, key('b'), Context::History, now),
            Some(Action::Home)
        );
        assert_eq!(
            resolver.feed(&bindings, key('b'), Context::Input, now),
            Some(Action::WordBegin)
        );
        assert_eq!(
            resolver.feed(&bindings, key('b'), Context::Visual, now),
            Some(Action::WordBegin)
        );
        assert_eq!(resolver.feed(&bindings, key('b'), Context::Pane, now), None);
        assert_eq!(
            resolver.feed(&bindings, key('g'), Context::Input, now),
            None
        );
        assert_eq!(
            resolver.feed(&bindings, key('g'), Context::Input, now),
            Some(Action::Home)
        );
        assert!(Keybindings::parse("[actions]\nword_begin=['b']\nword_end=['b']").is_err());
        let remapped =
            Keybindings::parse("[actions]\nword_begin=['Alt-b']\nword_end=['Alt-f']").unwrap();
        assert_eq!(
            resolver.feed(
                &remapped,
                KeyEvent::new(KeyCode::Char('f'), KeyModifiers::ALT),
                Context::Insert,
                now
            ),
            Some(Action::WordEnd)
        );
        assert_eq!(
            resolver.feed(&remapped, key('b'), Context::Insert, now),
            None
        );
    }
}
