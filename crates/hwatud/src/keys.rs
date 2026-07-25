// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (c) 2026 Justin Hong
//! Keybindings: every global key in hwatu maps to a named [`Action`]
//! through a [`Keymap`], so users can rebind anything.
//!
//! Defaults are vim-flavored (see [`Keymap::default`]); overrides live
//! in `~/.config/hwatu/keys.conf`, one `action = chord[, chord...]`
//! per line:
//!
//! ```text
//! # ~/.config/hwatu/keys.conf
//! back     = ctrl+o
//! forward  = ctrl+i
//! url_edit = ctrl+l, O
//! find     = slash
//! close    = none          # unbind an action entirely
//! ```
//!
//! Chords are `[ctrl+][alt+][shift+]key`. A key is a single character
//! (`o`, `/`) or a GDK key name (`slash`, `question`, `Left`). An
//! uppercase letter implies shift (`O` == `shift+o`). Assigning an
//! action replaces all of its default chords. The file is read once at
//! daemon startup.
//!
//! Dispatch phase is derived, not configured: chords with ctrl/alt run
//! in the GTK capture phase (they win over the page, address-bar
//! style), bare-key chords run in the bubble phase (an `o` typed into
//! a page's text box still reaches the page).

use glib::translate::FromGlib;
use gtk::gdk;

/// Everything a key can do. `name` strings are the config vocabulary.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    Close,
    UrlOpen,
    UrlEdit,
    Find,
    FindBack,
    FindNext,
    FindPrev,
    ScrollDown,
    ScrollUp,
    Back,
    Forward,
    Reload,
}

impl Action {
    const ALL: &'static [Action] = &[
        Action::Close,
        Action::UrlOpen,
        Action::UrlEdit,
        Action::Find,
        Action::FindBack,
        Action::FindNext,
        Action::FindPrev,
        Action::ScrollDown,
        Action::ScrollUp,
        Action::Back,
        Action::Forward,
        Action::Reload,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Action::Close => "close",
            Action::UrlOpen => "url_open",
            Action::UrlEdit => "url_edit",
            Action::Find => "find",
            Action::FindBack => "find_back",
            Action::FindNext => "find_next",
            Action::FindPrev => "find_prev",
            Action::ScrollDown => "scroll_down",
            Action::ScrollUp => "scroll_up",
            Action::Back => "back",
            Action::Forward => "forward",
            Action::Reload => "reload",
        }
    }

    /// Human-readable description, for the launcher page.
    pub fn describe(self) -> &'static str {
        match self {
            Action::Close => "close window",
            Action::UrlOpen => "open URL / search",
            Action::UrlEdit => "edit current URL",
            Action::Find => "find in page",
            Action::FindBack => "find backwards",
            Action::FindNext => "next match",
            Action::FindPrev => "previous match",
            Action::ScrollDown => "scroll down",
            Action::ScrollUp => "scroll up",
            Action::Back => "history back",
            Action::Forward => "history forward",
            Action::Reload => "reload page",
        }
    }

    fn from_name(name: &str) -> Option<Action> {
        Action::ALL.iter().copied().find(|a| a.name() == name)
    }

    /// Default chords, in config syntax.
    fn default_chords(self) -> &'static str {
        match self {
            Action::Close => "ctrl+w, ctrl+q",
            Action::UrlOpen => "o",
            Action::UrlEdit => "ctrl+l, O",
            Action::Find => "slash",
            Action::FindBack => "question",
            Action::FindNext => "n",
            Action::FindPrev => "N",
            Action::ScrollDown => "ctrl+shift+j",
            Action::ScrollUp => "ctrl+shift+k",
            Action::Back => "ctrl+o",
            Action::Forward => "ctrl+i",
            Action::Reload => "ctrl+r, F5",
        }
    }
}

/// Which key-controller a chord is matched in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    /// Before the WebView: modified chords that must beat the page.
    Capture,
    /// After the WebView declined the key: bare-key chords.
    Bubble,
}

/// One resolved key combination. `key` is stored lowercased; shift is
/// tracked explicitly so `N` and `n` are distinct while `?` (which
/// needs shift on most layouts) still matches without it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Chord {
    key: gdk::Key,
    ctrl: bool,
    shift: bool,
    alt: bool,
}

impl Chord {
    pub fn phase(&self) -> Phase {
        if self.ctrl || self.alt {
            Phase::Capture
        } else {
            Phase::Bubble
        }
    }

    /// Parse `[ctrl+][alt+][shift+]key`. Uppercase letters imply shift.
    fn parse(spec: &str) -> Result<Chord, String> {
        let mut ctrl = false;
        let mut shift = false;
        let mut alt = false;
        let parts: Vec<&str> = spec.split('+').map(str::trim).collect();
        let (mods, key_part) = parts.split_at(parts.len().saturating_sub(1));
        let key_part = key_part.first().copied().filter(|k| !k.is_empty());
        let Some(token) = key_part else {
            return Err(format!("empty key in {spec:?}"));
        };
        for m in mods {
            match m.to_ascii_lowercase().as_str() {
                "ctrl" | "control" => ctrl = true,
                "shift" => shift = true,
                "alt" | "meta" => alt = true,
                other => return Err(format!("unknown modifier {other:?} in {spec:?}")),
            }
        }

        // Single character: uppercase implies shift. Otherwise a GDK
        // key name (slash, question, Left, Page_Down, ...).
        let mut chars = token.chars();
        let key = match (chars.next(), chars.next()) {
            (Some(c), None) => {
                if c.is_uppercase() {
                    shift = true;
                }
                key_from_char(c.to_lowercase().next().unwrap_or(c))
            }
            _ => {
                let key = gdk::Key::from_name(token)
                    .ok_or_else(|| format!("unknown key name {token:?} in {spec:?}"))?;
                key.to_lower()
            }
        };
        Ok(Chord {
            key,
            ctrl,
            shift,
            alt,
        })
    }

    /// Does a GDK key event match this chord? Shift participates only
    /// for keys with letter case: `?` and `/` are distinct keyvals
    /// already, and requiring shift for `?` would depend on layout.
    fn matches(&self, key: gdk::Key, state: gdk::ModifierType) -> bool {
        if key.to_lower() != self.key {
            return false;
        }
        if state.contains(gdk::ModifierType::CONTROL_MASK) != self.ctrl {
            return false;
        }
        if state.contains(gdk::ModifierType::ALT_MASK) != self.alt {
            return false;
        }
        let cased = key.to_lower() != key.to_upper();
        if cased && state.contains(gdk::ModifierType::SHIFT_MASK) != self.shift {
            return false;
        }
        true
    }
}

/// Human rendering (`ctrl+J`, `O`, `/`, `Page_Down`), for the
/// launcher page's keybind table. Shift folds into letter case, as in
/// config syntax; printable keys show their character, others their
/// GDK name.
impl std::fmt::Display for Chord {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.ctrl {
            write!(f, "ctrl+")?;
        }
        if self.alt {
            write!(f, "alt+")?;
        }
        match self.key.to_unicode().filter(|c| !c.is_control()) {
            Some(c) if c.is_alphabetic() => {
                if self.shift {
                    write!(f, "{}", c.to_uppercase())
                } else {
                    write!(f, "{c}")
                }
            }
            Some(c) => write!(f, "{c}"),
            None => {
                if self.shift {
                    write!(f, "shift+")?;
                }
                write!(f, "{}", self.key.name().unwrap_or_default())
            }
        }
    }
}

/// Char -> GDK keyval, per gdk_unicode_to_keyval: Latin-1 code points
/// map directly, everything else is `codepoint | 0x0100_0000`.
fn key_from_char(c: char) -> gdk::Key {
    let cp = c as u32;
    let keyval = if cp <= 0xFF { cp } else { cp | 0x0100_0000 };
    unsafe { gdk::Key::from_glib(keyval) }
}

/// The full binding table, resolved at daemon startup.
pub struct Keymap {
    bindings: Vec<(Chord, Action)>,
}

impl Keymap {
    /// Built-in defaults with `keys.conf` overrides applied.
    pub fn load() -> Keymap {
        let mut map = Keymap::default();
        let Some(path) = config_file() else {
            return map;
        };
        let Ok(text) = std::fs::read_to_string(&path) else {
            return map;
        };
        let mut overrides = 0usize;
        for (lineno, line) in text.lines().enumerate() {
            let line = line.split('#').next().unwrap_or("").trim();
            if line.is_empty() {
                continue;
            }
            match map.apply_line(line) {
                Ok(()) => overrides += 1,
                Err(e) => eprintln!(
                    "hwatud: keys.conf:{}: {e}; keeping defaults for that line",
                    lineno + 1
                ),
            }
        }
        if overrides > 0 {
            println!(
                "hwatud: {overrides} key override(s) from {}",
                path.display()
            );
        }
        map
    }

    /// `action = chord[, chord...]` or `action = none`. Replaces all
    /// existing chords for that action.
    pub(crate) fn apply_line(&mut self, line: &str) -> Result<(), String> {
        let (name, spec) = line
            .split_once('=')
            .ok_or_else(|| format!("expected `action = chord`, got {line:?}"))?;
        let name = name.trim();
        let action = Action::from_name(name).ok_or_else(|| {
            let known: Vec<&str> = Action::ALL.iter().map(|a| a.name()).collect();
            format!("unknown action {name:?} (known: {})", known.join(", "))
        })?;
        let spec = spec.trim();
        let chords = if spec.eq_ignore_ascii_case("none") {
            Vec::new()
        } else {
            spec.split(',')
                .map(|s| Chord::parse(s.trim()))
                .collect::<Result<Vec<_>, _>>()?
        };
        self.bindings.retain(|(_, a)| *a != action);
        self.bindings
            .extend(chords.into_iter().map(|c| (c, action)));
        Ok(())
    }

    /// First action bound to this event in the given phase.
    pub fn lookup(&self, phase: Phase, key: gdk::Key, state: gdk::ModifierType) -> Option<Action> {
        self.bindings
            .iter()
            .find(|(chord, _)| chord.phase() == phase && chord.matches(key, state))
            .map(|(_, action)| *action)
    }

    /// Every chord bound to `action`, rendered in config syntax and in
    /// binding order. Empty if unbound.
    pub fn chords_for(&self, action: Action) -> Vec<String> {
        self.bindings
            .iter()
            .filter(|(_, a)| *a == action)
            .map(|(chord, _)| chord.to_string())
            .collect()
    }
}

impl Default for Keymap {
    fn default() -> Keymap {
        let mut bindings = Vec::new();
        for action in Action::ALL {
            for spec in action.default_chords().split(',') {
                let chord = Chord::parse(spec.trim())
                    .unwrap_or_else(|e| panic!("bad built-in chord for {action:?}: {e}"));
                bindings.push((chord, *action));
            }
        }
        Keymap { bindings }
    }
}

/// `~/.config/hwatu/keys.conf` (honoring `XDG_CONFIG_HOME`).
fn config_file() -> Option<std::path::PathBuf> {
    let base = std::env::var("XDG_CONFIG_HOME")
        .ok()
        .filter(|v| !v.is_empty())
        .map(std::path::PathBuf::from)
        .or_else(|| {
            std::env::var("HOME")
                .ok()
                .map(|h| std::path::PathBuf::from(h).join(".config"))
        })?;
    Some(base.join("hwatu").join("keys.conf"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use gtk::gdk::{Key, ModifierType};

    const NONE: ModifierType = ModifierType::empty();
    const CTRL: ModifierType = ModifierType::CONTROL_MASK;
    const SHIFT: ModifierType = ModifierType::SHIFT_MASK;

    #[test]
    fn chords_render_for_humans() {
        let show = |spec: &str| Chord::parse(spec).unwrap().to_string();
        assert_eq!(show("o"), "o");
        assert_eq!(show("O"), "O");
        assert_eq!(show("ctrl+l"), "ctrl+l");
        assert_eq!(show("ctrl+shift+j"), "ctrl+J");
        assert_eq!(show("slash"), "/");
        assert_eq!(show("question"), "?");
        assert_eq!(show("Page_Down"), "Page_Down");
    }

    #[test]
    fn chords_for_lists_bindings_in_order() {
        let map = Keymap::default();
        assert_eq!(map.chords_for(Action::UrlEdit), vec!["ctrl+l", "O"]);
        assert!(map.chords_for(Action::Find).contains(&"/".to_string()));
    }

    #[test]
    fn defaults_resolve() {
        let map = Keymap::default();
        assert_eq!(map.lookup(Phase::Capture, Key::o, CTRL), Some(Action::Back));
        assert_eq!(
            map.lookup(Phase::Capture, Key::i, CTRL),
            Some(Action::Forward)
        );
        assert_eq!(
            map.lookup(Phase::Capture, Key::l, CTRL),
            Some(Action::UrlEdit)
        );
        assert_eq!(
            map.lookup(Phase::Bubble, Key::o, NONE),
            Some(Action::UrlOpen)
        );
        // With Shift held GTK reports the uppercased keyval.
        assert_eq!(
            map.lookup(Phase::Bubble, Key::O, SHIFT),
            Some(Action::UrlEdit)
        );
        assert_eq!(
            map.lookup(Phase::Bubble, Key::n, NONE),
            Some(Action::FindNext)
        );
        assert_eq!(
            map.lookup(Phase::Bubble, Key::N, SHIFT),
            Some(Action::FindPrev)
        );
        // `?` arrives with shift set on most layouts; it matches anyway.
        assert_eq!(
            map.lookup(Phase::Bubble, Key::question, SHIFT),
            Some(Action::FindBack)
        );
        assert_eq!(
            map.lookup(Phase::Capture, Key::J, CTRL | SHIFT),
            Some(Action::ScrollDown)
        );
        assert_eq!(
            map.lookup(Phase::Capture, Key::w, CTRL),
            Some(Action::Close)
        );
        assert_eq!(
            map.lookup(Phase::Capture, Key::q, CTRL),
            Some(Action::Close)
        );
        assert_eq!(
            map.lookup(Phase::Capture, Key::r, CTRL),
            Some(Action::Reload)
        );
        // F5 carries no modifiers, so it bubbles like other bare keys.
        assert_eq!(
            map.lookup(Phase::Bubble, Key::F5, NONE),
            Some(Action::Reload)
        );
    }

    #[test]
    fn phases_are_derived_from_modifiers() {
        // ctrl+o is capture-only; bare o is bubble-only.
        let map = Keymap::default();
        assert_eq!(map.lookup(Phase::Bubble, Key::o, CTRL), None);
        assert_eq!(map.lookup(Phase::Capture, Key::o, NONE), None);
    }

    #[test]
    fn overrides_replace_defaults() {
        let mut map = Keymap::default();
        map.apply_line("back = alt+Left").unwrap();
        assert_eq!(map.lookup(Phase::Capture, Key::o, CTRL), None);
        assert_eq!(
            map.lookup(Phase::Capture, Key::Left, ModifierType::ALT_MASK),
            Some(Action::Back)
        );
    }

    #[test]
    fn none_unbinds() {
        let mut map = Keymap::default();
        map.apply_line("close = none").unwrap();
        assert_eq!(map.lookup(Phase::Capture, Key::q, CTRL), None);
    }

    #[test]
    fn multiple_chords_per_action() {
        let mut map = Keymap::default();
        map.apply_line("forward = ctrl+i, alt+Right").unwrap();
        assert_eq!(
            map.lookup(Phase::Capture, Key::i, CTRL),
            Some(Action::Forward)
        );
        assert_eq!(
            map.lookup(Phase::Capture, Key::Right, ModifierType::ALT_MASK),
            Some(Action::Forward)
        );
    }

    #[test]
    fn bad_lines_error_and_leave_map_alone() {
        let mut map = Keymap::default();
        assert!(map.apply_line("nonsense = ctrl+x").is_err());
        assert!(map.apply_line("back ctrl+o").is_err());
        assert!(map.apply_line("back = hyper+o").is_err());
        assert!(map.apply_line("back = notakeyname").is_err());
        // back still bound after all the failures
        assert_eq!(map.lookup(Phase::Capture, Key::o, CTRL), Some(Action::Back));
    }

    #[test]
    fn uppercase_letter_implies_shift() {
        let chord = Chord::parse("O").unwrap();
        assert!(chord.shift);
        assert_eq!(chord.phase(), Phase::Bubble);
        let chord = Chord::parse("ctrl+shift+j").unwrap();
        assert!(chord.ctrl && chord.shift);
        assert_eq!(chord.phase(), Phase::Capture);
    }
}

#[cfg(test)]
mod char_name_tests {
    use super::*;

    #[test]
    fn punctuation_chars_parse() {
        // Docs promise `/` and `?` work as bare characters.
        assert!(Chord::parse("/").is_ok());
        assert!(Chord::parse("?").is_ok());
        let mut map = Keymap::default();
        map.apply_line("find = /").unwrap();
        assert_eq!(
            map.lookup(
                Phase::Bubble,
                gtk::gdk::Key::slash,
                gtk::gdk::ModifierType::empty()
            ),
            Some(Action::Find)
        );
    }
}
