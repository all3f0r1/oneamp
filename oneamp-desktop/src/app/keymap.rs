//! The one table of keyboard shortcuts. Each binding maps a key chord,
//! in a focus scope, to a `Command`; most commands wrap the
//! `MainWindowAction` the menus and skin buttons already emit, so a
//! shortcut does exactly what the matching menu entry does. The F1
//! cheat-sheet is generated from this table.

use crate::windows::MainWindowAction as A;
use egui::Key;
use serde::{Deserialize, Serialize};

/// Which keyboard layout is active. Classic follows Winamp 2.x / 5;
/// Legacy keeps OneAmp 1.0's keys (N/P/S transport, letter type-to-jump
/// in the playlist).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum KeyProfile {
    #[default]
    WinampClassic,
    OneAmpLegacy,
}

/// Where a binding applies. Focused-window bindings win over global ones.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scope {
    Global,
    Playlist,
    Equalizer,
}

#[derive(Debug, Clone)]
pub enum Command {
    Action(A),
    Prev,
    Play,
    Pause,
    PlayPause,
    Stop,
    Next,
    VolumeUp { fine: bool },
    VolumeDown { fine: bool },
    SeekForward { long: bool },
    SeekBack { long: bool },
    Mute,
    JumpToFile,
    OpenUrl,
    FilterPlaylist,
    Preferences,
    ToggleTimeDisplay,
    Undo,
    // Playlist window
    SelectBy { delta: isize, extend: bool },
    SelectEdge { end: bool },
    MoveSelection { up: bool },
    PlaySelected,
    RemoveSelected,
    SelectAll,
    QueueSelected,
    // Equalizer window
    EqBand { band: usize, up: bool },
    EqPreamp { up: bool },
    EqToggle,
    EqAuto,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Chord {
    pub key: Key,
    pub ctrl: bool,
    pub shift: bool,
    pub alt: bool,
}

const fn k(key: Key) -> Chord {
    Chord {
        key,
        ctrl: false,
        shift: false,
        alt: false,
    }
}
const fn ctrl(key: Key) -> Chord {
    Chord {
        ctrl: true,
        ..k(key)
    }
}
const fn shift(key: Key) -> Chord {
    Chord {
        shift: true,
        ..k(key)
    }
}
const fn alt(key: Key) -> Chord {
    Chord {
        alt: true,
        ..k(key)
    }
}
const fn ctrl_shift(key: Key) -> Chord {
    Chord {
        ctrl: true,
        shift: true,
        ..k(key)
    }
}

pub struct Binding {
    pub chord: Chord,
    pub scope: Scope,
    pub command: Command,
    /// Cheat-sheet label; bindings sharing a label are listed together.
    pub label: &'static str,
}

fn b(chord: Chord, scope: Scope, command: Command, label: &'static str) -> Binding {
    Binding {
        chord,
        scope,
        command,
        label,
    }
}

const EQ_UP_KEYS: [Key; 10] = [
    Key::Num1,
    Key::Num2,
    Key::Num3,
    Key::Num4,
    Key::Num5,
    Key::Num6,
    Key::Num7,
    Key::Num8,
    Key::Num9,
    Key::Num0,
];
const EQ_DOWN_KEYS: [Key; 10] = [
    Key::Q,
    Key::W,
    Key::E,
    Key::R,
    Key::T,
    Key::Y,
    Key::U,
    Key::I,
    Key::O,
    Key::P,
];

/// Bindings shared by both profiles: windows, files, navigation.
fn common() -> Vec<Binding> {
    use Command as C;
    use Scope::*;
    let mut v = vec![
        b(k(Key::Z), Global, C::Prev, "Previous track"),
        b(k(Key::X), Global, C::Play, "Play / restart"),
        b(k(Key::C), Global, C::Pause, "Pause"),
        b(k(Key::V), Global, C::Stop, "Stop"),
        b(k(Key::B), Global, C::Next, "Next track"),
        b(k(Key::Space), Global, C::PlayPause, "Play / pause"),
        b(
            k(Key::ArrowUp),
            Global,
            C::VolumeUp { fine: false },
            "Volume up",
        ),
        b(
            shift(Key::ArrowUp),
            Global,
            C::VolumeUp { fine: true },
            "Volume up",
        ),
        b(
            k(Key::ArrowDown),
            Global,
            C::VolumeDown { fine: false },
            "Volume down",
        ),
        b(
            shift(Key::ArrowDown),
            Global,
            C::VolumeDown { fine: true },
            "Volume down",
        ),
        b(
            k(Key::ArrowRight),
            Global,
            C::SeekForward { long: false },
            "Seek +5 s (Shift +30 s)",
        ),
        b(
            shift(Key::ArrowRight),
            Global,
            C::SeekForward { long: true },
            "Seek +5 s (Shift +30 s)",
        ),
        b(
            k(Key::ArrowLeft),
            Global,
            C::SeekBack { long: false },
            "Seek -5 s (Shift -30 s)",
        ),
        b(
            shift(Key::ArrowLeft),
            Global,
            C::SeekBack { long: true },
            "Seek -5 s (Shift -30 s)",
        ),
        b(k(Key::L), Global, C::Action(A::OpenFile), "Open file"),
        b(ctrl(Key::O), Global, C::Action(A::OpenFile), "Open file"),
        b(
            shift(Key::L),
            Global,
            C::Action(A::OpenFolder),
            "Open folder",
        ),
        b(
            ctrl_shift(Key::O),
            Global,
            C::Action(A::OpenFolder),
            "Open folder",
        ),
        b(ctrl(Key::L), Global, C::OpenUrl, "Open URL"),
        b(k(Key::J), Global, C::JumpToFile, "Jump to file"),
        b(k(Key::F3), Global, C::JumpToFile, "Jump to file"),
        b(ctrl(Key::F), Global, C::FilterPlaylist, "Filter playlist"),
        b(ctrl(Key::Z), Global, C::Undo, "Undo playlist change"),
        b(
            alt(Key::E),
            Global,
            C::Action(A::TogglePlaylist),
            "Show / hide playlist",
        ),
        b(
            alt(Key::G),
            Global,
            C::Action(A::ToggleEqualizer),
            "Show / hide equalizer",
        ),
        b(ctrl(Key::P), Global, C::Preferences, "Preferences"),
        b(alt(Key::S), Global, C::Action(A::PickSkin), "Skins"),
        b(
            ctrl(Key::D),
            Global,
            C::Action(A::ToggleDoubleSize),
            "Double size",
        ),
        b(
            k(Key::F1),
            Global,
            C::Action(A::ShowHotkeys),
            "Keyboard shortcuts",
        ),
        // Playlist window
        b(
            k(Key::ArrowUp),
            Playlist,
            C::SelectBy {
                delta: -1,
                extend: false,
            },
            "Move selection",
        ),
        b(
            k(Key::ArrowDown),
            Playlist,
            C::SelectBy {
                delta: 1,
                extend: false,
            },
            "Move selection",
        ),
        b(
            shift(Key::ArrowUp),
            Playlist,
            C::SelectBy {
                delta: -1,
                extend: true,
            },
            "Extend selection",
        ),
        b(
            shift(Key::ArrowDown),
            Playlist,
            C::SelectBy {
                delta: 1,
                extend: true,
            },
            "Extend selection",
        ),
        b(
            k(Key::PageUp),
            Playlist,
            C::SelectBy {
                delta: -10,
                extend: false,
            },
            "Page up / down",
        ),
        b(
            k(Key::PageDown),
            Playlist,
            C::SelectBy {
                delta: 10,
                extend: false,
            },
            "Page up / down",
        ),
        b(
            k(Key::Home),
            Playlist,
            C::SelectEdge { end: false },
            "First / last entry",
        ),
        b(
            k(Key::End),
            Playlist,
            C::SelectEdge { end: true },
            "First / last entry",
        ),
        b(
            alt(Key::ArrowUp),
            Playlist,
            C::MoveSelection { up: true },
            "Move selected entries",
        ),
        b(
            alt(Key::ArrowDown),
            Playlist,
            C::MoveSelection { up: false },
            "Move selected entries",
        ),
        b(
            k(Key::Enter),
            Playlist,
            C::PlaySelected,
            "Play selected entry",
        ),
        b(
            k(Key::Delete),
            Playlist,
            C::RemoveSelected,
            "Remove selected entries",
        ),
        b(ctrl(Key::A), Playlist, C::SelectAll, "Select all"),
        b(
            k(Key::Q),
            Playlist,
            C::QueueSelected,
            "Queue / unqueue selected",
        ),
        // Equalizer window
        b(
            k(Key::Backtick),
            Equalizer,
            C::EqPreamp { up: true },
            "Preamp up / down",
        ),
        b(
            k(Key::Tab),
            Equalizer,
            C::EqPreamp { up: false },
            "Preamp up / down",
        ),
        b(k(Key::N), Equalizer, C::EqToggle, "Equalizer on / off"),
        b(k(Key::A), Equalizer, C::EqAuto, "EQ AUTO on / off"),
    ];
    for band in 0..10 {
        v.push(b(
            k(EQ_UP_KEYS[band]),
            Equalizer,
            C::EqBand { band, up: true },
            "EQ band up: 1-0",
        ));
        v.push(b(
            k(EQ_DOWN_KEYS[band]),
            Equalizer,
            C::EqBand { band, up: false },
            "EQ band down: Q-P",
        ));
    }
    v
}

/// Full binding table for a profile.
pub fn bindings(profile: KeyProfile) -> Vec<Binding> {
    use Command as C;
    use Scope::*;
    let mut v = common();
    match profile {
        KeyProfile::WinampClassic => v.extend([
            b(
                k(Key::S),
                Global,
                C::Action(A::ToggleShuffle),
                "Shuffle on / off",
            ),
            b(k(Key::R), Global, C::Action(A::CycleRepeat), "Repeat mode"),
            b(
                ctrl(Key::V),
                Global,
                C::Action(A::ToggleStopAfterCurrent),
                "Stop after current",
            ),
            b(
                ctrl(Key::W),
                Global,
                C::Action(A::ToggleShade),
                "Window shade",
            ),
            b(
                ctrl(Key::A),
                Global,
                C::Action(A::ToggleAlwaysOnTop),
                "Always on top",
            ),
            b(
                ctrl(Key::T),
                Global,
                C::ToggleTimeDisplay,
                "Elapsed / remaining time",
            ),
        ]),
        KeyProfile::OneAmpLegacy => v.extend([
            b(k(Key::N), Global, C::Next, "Next track"),
            b(k(Key::P), Global, C::Prev, "Previous track"),
            b(k(Key::S), Global, C::Stop, "Stop"),
            b(
                shift(Key::S),
                Global,
                C::Action(A::ToggleStopAfterCurrent),
                "Stop after current",
            ),
            b(k(Key::R), Global, C::Action(A::CycleRepeat), "Repeat mode"),
            b(k(Key::M), Global, C::Mute, "Mute"),
            b(
                alt(Key::M),
                Global,
                C::Action(A::ToggleShade),
                "Window shade",
            ),
            b(
                ctrl(Key::T),
                Global,
                C::Action(A::ToggleAlwaysOnTop),
                "Always on top",
            ),
        ]),
    }
    v
}

/// Command for `chord` given the focused window: its scope first, then
/// global bindings.
pub fn resolve(table: &[Binding], chord: Chord, focused: Scope) -> Option<&Command> {
    let find = |scope| {
        table
            .iter()
            .find(|b| b.scope == scope && b.chord == chord)
            .map(|b| &b.command)
    };
    if focused != Scope::Global
        && let Some(c) = find(focused)
    {
        return Some(c);
    }
    find(Scope::Global)
}

/// Human-readable chord, e.g. `Ctrl+Shift+O`.
pub fn chord_label(c: Chord) -> String {
    let mut s = String::new();
    if c.ctrl {
        s.push_str("Ctrl+");
    }
    if c.alt {
        s.push_str("Alt+");
    }
    if c.shift {
        s.push_str("Shift+");
    }
    s.push_str(match c.key {
        Key::ArrowUp => "Up",
        Key::ArrowDown => "Down",
        Key::ArrowLeft => "Left",
        Key::ArrowRight => "Right",
        Key::Backtick => "`",
        other => other.name(),
    });
    s
}

/// Cheat-sheet rows `(scope, keys, label)`, one per distinct label and
/// scope, keys joined in table order.
pub fn help_rows(profile: KeyProfile) -> Vec<(Scope, String, &'static str)> {
    let mut rows: Vec<(Scope, String, &'static str)> = Vec::new();
    for bnd in bindings(profile) {
        // EQ band rows already name their keys in the label.
        let keys = if matches!(bnd.command, Command::EqBand { .. }) {
            String::new()
        } else {
            chord_label(bnd.chord)
        };
        match rows
            .iter_mut()
            .find(|(s, _, l)| *s == bnd.scope && *l == bnd.label)
        {
            Some((_, existing, _)) if !keys.is_empty() && !existing.contains(&keys) => {
                existing.push_str(" / ");
                existing.push_str(&keys);
            }
            Some(_) => {}
            None => rows.push((bnd.scope, keys, bnd.label)),
        }
    }
    rows
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_chord_is_bound_twice_in_one_scope() {
        for profile in [KeyProfile::WinampClassic, KeyProfile::OneAmpLegacy] {
            let t = bindings(profile);
            for (i, a) in t.iter().enumerate() {
                for bnd in &t[i + 1..] {
                    assert!(
                        !(a.scope == bnd.scope && a.chord == bnd.chord),
                        "{profile:?}: {} bound twice in {:?}",
                        chord_label(a.chord),
                        a.scope
                    );
                }
            }
        }
    }

    #[test]
    fn focused_scope_wins_then_global() {
        let t = bindings(KeyProfile::WinampClassic);
        // Up: volume globally, selection in the playlist.
        let r = |c, s| resolve(&t, c, s);
        assert!(matches!(
            r(k(Key::ArrowUp), Scope::Global),
            Some(Command::VolumeUp { fine: false })
        ));
        assert!(matches!(
            r(k(Key::ArrowUp), Scope::Playlist),
            Some(Command::SelectBy {
                delta: -1,
                extend: false
            })
        ));
        // X plays from the playlist too (Winamp), R lowers band 4 in the EQ.
        assert!(matches!(r(k(Key::X), Scope::Playlist), Some(Command::Play)));
        assert!(matches!(
            r(k(Key::R), Scope::Equalizer),
            Some(Command::EqBand { band: 3, up: false })
        ));
        // J and F3 both open Jump to file.
        assert!(matches!(
            r(k(Key::F3), Scope::Playlist),
            Some(Command::JumpToFile)
        ));
        assert_eq!(chord_label(ctrl_shift(Key::O)), "Ctrl+Shift+O");
    }
}
