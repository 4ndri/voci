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
impl Action {
    fn contexts(self) -> u8 {
        match self {
            Self::Filter | Self::Refresh => 2,
            Self::Edit => 1,
            _ => 3,
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Key {
    code: KeyCode,
    modifiers: KeyModifiers,
}
impl Key {
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
                bindings.push(Binding {
                    action,
                    keys,
                    label,
                });
            }
        }
        if !profile.navigation.is_empty() || !profile.actions.is_empty() {
            return Err(
                "Unknown keybinding action; check [navigation] and [actions] names.".into(),
            );
        }
        for (i, a) in bindings.iter().enumerate() {
            for b in &bindings[i + 1..] {
                if a.action.contexts() & b.action.contexts() != 0
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
    pub fn direct(&self, key: KeyEvent, action: Action) -> bool {
        let key = Key::event(key);
        self.bindings
            .iter()
            .any(|b| b.action == action && b.keys == [key])
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
    pane: bool,
}
impl Resolver {
    pub fn reset(&mut self) {
        self.pending.clear();
        self.last = None;
        self.pane = false;
    }
    pub fn pending(&self) -> bool {
        (!self.pending.is_empty() || self.pane)
            && self
                .last
                .is_some_and(|last| last.elapsed() < Duration::from_millis(750))
    }
    pub fn feed(
        &mut self,
        bindings: &Keybindings,
        event: KeyEvent,
        history: bool,
        now: Instant,
    ) -> Option<(Action, bool)> {
        let context = if history { 2 } else { 1 };
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
                .filter(|b| b.action.contexts() & context != 0 && b.keys.starts_with(keys))
                .collect::<Vec<_>>()
        };
        let mut matches = candidates(&self.pending);
        if matches.is_empty() {
            self.pending = vec![Key::event(event)];
            self.pane = false;
            matches = candidates(&self.pending);
        }
        if let Some(binding) = matches.iter().find(|b| b.keys == self.pending) {
            let action = binding.action;
            self.pending.clear();
            if action == Action::Pane {
                self.pane = true;
                return None;
            }
            let pane = self.pane;
            self.pane = false;
            if pane
                && !matches!(
                    action,
                    Action::Left | Action::Right | Action::Up | Action::Down
                )
            {
                return None;
            }
            return Some((action, pane));
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
        assert_eq!(r.feed(&b, key('g'), true, now), None);
        assert_eq!(r.feed(&b, key('g'), true, now), Some((Action::Home, false)));
        assert_eq!(r.feed(&b, key('b'), true, now), Some((Action::Home, false)));
        assert_eq!(r.feed(&b, key('l'), true, now), Some((Action::End, false)));
        assert_eq!(r.feed(&b, key('G'), true, now), Some((Action::End, false)));
        assert_eq!(r.feed(&b, key('h'), true, now), None);
        assert_eq!(
            r.feed(
                &b,
                KeyEvent::new(KeyCode::Char('w'), KeyModifiers::CONTROL),
                true,
                now
            ),
            None
        );
        assert_eq!(r.feed(&b, key('t'), true, now), Some((Action::Left, true)));
        r.feed(&b, key('g'), true, now);
        assert_eq!(
            r.feed(&b, key('t'), true, now + Duration::from_secs(1)),
            Some((Action::Left, false))
        );
        assert!(Keybindings::parse("[navigation]\nhome=['g']").is_err());
        assert!(Keybindings::parse("[navigation]\nleft=['i']").is_err());
        assert!(Keybindings::parse("[navigation]\nleft=[]").is_err());
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
}
