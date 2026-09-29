// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Justin Hong
//! Portable engine vocabulary (P2a of the EngineBackend plan, see
//! docs/architecture-audit.md).
//!
//! Engine-neutral types for concepts that core modules previously
//! expressed with `gdk`/`webkit6` types directly. Conversions to the
//! WebKitGTK representations live here too, so the *only* place that
//! names both vocabularies is this file. As the EngineBackend trait
//! lands (P2b), these become the trait's parameter types and the
//! `to_gtk`/`from_gtk` conversions move into the WebKitGTK backend impl.

#![allow(dead_code)]

/// Page-load lifecycle stages, engine-neutral mirror of
/// `webkit6::LoadEvent` (and semantically of `hwatu_ipc::LoadStage`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoadPhase {
    Started,
    Redirected,
    Committed,
    Finished,
}

impl From<webkit6::LoadEvent> for LoadPhase {
    fn from(e: webkit6::LoadEvent) -> Self {
        match e {
            webkit6::LoadEvent::Started => LoadPhase::Started,
            webkit6::LoadEvent::Redirected => LoadPhase::Redirected,
            webkit6::LoadEvent::Committed => LoadPhase::Committed,
            _ => LoadPhase::Finished,
        }
    }
}

/// What part of the page a snapshot covers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SnapshotArea {
    /// The currently visible viewport.
    Visible,
    /// The full document, beyond the viewport.
    FullDocument,
}

impl SnapshotArea {
    pub fn from_full_page(full_page: bool) -> Self {
        if full_page {
            SnapshotArea::FullDocument
        } else {
            SnapshotArea::Visible
        }
    }

    pub fn to_webkit(self) -> webkit6::SnapshotRegion {
        match self {
            SnapshotArea::Visible => webkit6::SnapshotRegion::Visible,
            SnapshotArea::FullDocument => webkit6::SnapshotRegion::FullDocument,
        }
    }
}

/// Engine-neutral keyboard modifiers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Modifiers {
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
    pub meta: bool,
}

impl Modifiers {
    pub fn from_gdk(state: gtk::gdk::ModifierType) -> Self {
        use gtk::gdk::ModifierType as M;
        Modifiers {
            ctrl: state.contains(M::CONTROL_MASK),
            alt: state.contains(M::ALT_MASK),
            shift: state.contains(M::SHIFT_MASK),
            meta: state.contains(M::META_MASK) || state.contains(M::SUPER_MASK),
        }
    }
}

/// Engine-neutral key identity: a Unicode character where one exists,
/// otherwise a named key. Deliberately not a full scancode model; hwatu
/// only needs enough identity for keybinding lookup and injected input.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum KeyIdent {
    /// A key that produces this character (case-significant).
    Char(char),
    /// A non-character key by canonical lowercase name, e.g. "escape",
    /// "return", "tab", "f5", "up".
    Named(&'static str),
    /// A named key not in the static table (owned fallback).
    Other(String),
}

impl KeyIdent {
    /// Engine-neutral parse of a key token as used in keybinding specs
    /// ("a", "Escape", "F5"). Mirrors `gdk::Key::from_name` semantics
    /// closely enough for hwatu's binding tables.
    pub fn from_gdk(key: gtk::gdk::Key) -> Self {
        if let Some(c) = key.to_unicode() {
            if !c.is_control() {
                return KeyIdent::Char(c);
            }
        }
        match key.name() {
            Some(name) => KeyIdent::Other(name.to_ascii_lowercase()),
            None => KeyIdent::Other(String::new()),
        }
    }
}
