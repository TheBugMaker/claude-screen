//! Key and mouse events as the bytes an xterm sends for them.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEventKind};
use vt100::MouseProtocolMode;

const ESC: u8 = 0x1b;

/// The bytes for `key`. `application_cursor` is the program's DECCKM mode, in which unmodified
/// arrows, Home and End use SS3 (`ESC O A`) instead of CSI (`ESC [ A`). Empty for keys that
/// send nothing.
#[must_use]
pub fn encode(key: KeyEvent, application_cursor: bool) -> Vec<u8> {
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    let alt = key.modifiers.contains(KeyModifiers::ALT);
    let shift = key.modifiers.contains(KeyModifiers::SHIFT);
    // xterm's modifier parameter: 1 + shift + 2·alt + 4·ctrl.
    let param = 1 + u8::from(shift) + 2 * u8::from(alt) + 4 * u8::from(ctrl);

    let bytes = match key.code {
        KeyCode::Char(c) => match control_byte(c).filter(|_| ctrl) {
            Some(byte) => vec![byte],
            None => c.encode_utf8(&mut [0; 4]).as_bytes().to_vec(),
        },
        // Plain CR would submit; ESC CR is the newline Claude Code and readline take.
        KeyCode::Enter if shift => vec![ESC, b'\r'],
        KeyCode::Enter => vec![b'\r'],
        KeyCode::Tab => vec![b'\t'],
        KeyCode::BackTab => b"\x1b[Z".to_vec(),
        KeyCode::Backspace if ctrl => vec![0x08],
        KeyCode::Backspace => vec![0x7f],
        KeyCode::Esc => vec![ESC],
        KeyCode::Up => cursor(b'A', param, application_cursor),
        KeyCode::Down => cursor(b'B', param, application_cursor),
        KeyCode::Right => cursor(b'C', param, application_cursor),
        KeyCode::Left => cursor(b'D', param, application_cursor),
        KeyCode::Home => cursor(b'H', param, application_cursor),
        KeyCode::End => cursor(b'F', param, application_cursor),
        KeyCode::Insert => tilde(2, param),
        KeyCode::Delete => tilde(3, param),
        KeyCode::PageUp => tilde(5, param),
        KeyCode::PageDown => tilde(6, param),
        KeyCode::F(n @ 1..=4) => {
            let letter = b"PQRS"[usize::from(n - 1)];
            if param > 1 {
                format!("\x1b[1;{param}{}", char::from(letter)).into_bytes()
            } else {
                vec![ESC, b'O', letter]
            }
        }
        KeyCode::F(n @ 5..=12) => {
            tilde([15, 17, 18, 19, 20, 21, 23, 24][usize::from(n - 5)], param)
        }
        _ => return Vec::new(),
    };
    // Alt is an ESC prefix, except on keys whose sequence carries it in `param`.
    let prefixed = matches!(
        key.code,
        KeyCode::Char(_) | KeyCode::Enter | KeyCode::Tab | KeyCode::Backspace | KeyCode::Esc
    );
    if alt && prefixed {
        [&[ESC][..], &bytes].concat()
    } else {
        bytes
    }
}

/// The SGR report (`ESC [ < b ; x ; y M`) of a mouse event at `column`, `row` (from 0) of the
/// program's screen. Empty when `mode`, what the program asked to hear, leaves it out.
#[must_use]
pub fn mouse(
    kind: MouseEventKind,
    modifiers: KeyModifiers,
    (column, row): (u16, u16),
    mode: MouseProtocolMode,
) -> Vec<u8> {
    use MouseProtocolMode as M;
    let button = |button: MouseButton| match button {
        MouseButton::Left => 0,
        MouseButton::Middle => 1,
        MouseButton::Right => 2,
    };
    let (code, pressed, wanted) = match kind {
        MouseEventKind::Down(b) => (button(b), true, mode != M::None),
        MouseEventKind::Up(b) => (button(b), false, mode != M::None && mode != M::Press),
        MouseEventKind::Drag(b) => (
            32 + button(b),
            true,
            matches!(mode, M::ButtonMotion | M::AnyMotion),
        ),
        MouseEventKind::Moved => (35, true, mode == M::AnyMotion),
        MouseEventKind::ScrollUp => (64, true, mode != M::None),
        MouseEventKind::ScrollDown => (65, true, mode != M::None),
        MouseEventKind::ScrollLeft => (66, true, mode != M::None),
        MouseEventKind::ScrollRight => (67, true, mode != M::None),
    };
    if !wanted {
        return Vec::new();
    }
    let code = code
        + 4 * u8::from(modifiers.contains(KeyModifiers::SHIFT))
        + 8 * u8::from(modifiers.contains(KeyModifiers::ALT))
        + 16 * u8::from(modifiers.contains(KeyModifiers::CONTROL));
    let end = if pressed { 'M' } else { 'm' };
    format!("\x1b[<{code};{};{}{end}", column + 1, row + 1).into_bytes()
}

/// The byte Ctrl+`c` sends, as in xterm.
fn control_byte(c: char) -> Option<u8> {
    let byte = match c.to_ascii_lowercase() {
        c @ 'a'..='z' => c as u8 & 0x1f,
        '@' | ' ' | '2' => 0,
        '[' | '3' => 0x1b,
        '\\' | '4' => 0x1c,
        ']' | '5' => 0x1d,
        '^' | '6' => 0x1e,
        '_' | '-' | '7' => 0x1f,
        '?' | '8' => 0x7f,
        _ => return None,
    };
    Some(byte)
}

fn cursor(letter: u8, param: u8, application_cursor: bool) -> Vec<u8> {
    if param > 1 {
        format!("\x1b[1;{param}{}", char::from(letter)).into_bytes()
    } else if application_cursor {
        vec![ESC, b'O', letter]
    } else {
        vec![ESC, b'[', letter]
    }
}

fn tilde(number: u8, param: u8) -> Vec<u8> {
    if param > 1 {
        format!("\x1b[{number};{param}~").into_bytes()
    } else {
        format!("\x1b[{number}~").into_bytes()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(code: KeyCode, modifiers: KeyModifiers) -> KeyEvent {
        KeyEvent::new(code, modifiers)
    }

    #[test]
    fn reports_mouse_events_the_program_asked_for() {
        let none = KeyModifiers::NONE;
        let left = MouseButton::Left;
        let any = MouseProtocolMode::AnyMotion;
        assert_eq!(
            mouse(MouseEventKind::ScrollUp, none, (4, 9), any),
            b"\x1b[<64;5;10M"
        );
        assert_eq!(
            mouse(MouseEventKind::Up(left), KeyModifiers::CONTROL, (0, 0), any),
            b"\x1b[<16;1;1m"
        );
        assert_eq!(
            mouse(MouseEventKind::Drag(left), none, (1, 1), any),
            b"\x1b[<32;2;2M"
        );
        // Presses only: no release, no motion.
        let press = MouseProtocolMode::Press;
        assert_eq!(
            mouse(MouseEventKind::Down(left), none, (0, 0), press),
            b"\x1b[<0;1;1M"
        );
        assert_eq!(mouse(MouseEventKind::Up(left), none, (0, 0), press), b"");
        assert_eq!(mouse(MouseEventKind::Drag(left), none, (0, 0), press), b"");
    }

    #[test]
    fn encodes_keys_like_xterm() {
        let none = KeyModifiers::NONE;
        let ctrl = KeyModifiers::CONTROL;
        let alt = KeyModifiers::ALT;
        let cases: [(KeyEvent, bool, &[u8]); 19] = [
            (key(KeyCode::Char('a'), none), false, b"a"),
            (key(KeyCode::Char('é'), none), false, "é".as_bytes()),
            (key(KeyCode::Char('A'), KeyModifiers::SHIFT), false, b"A"),
            (key(KeyCode::Char('c'), ctrl), false, b"\x03"),
            (key(KeyCode::Char(' '), ctrl), false, b"\0"),
            (key(KeyCode::Char('['), ctrl), false, b"\x1b"),
            (key(KeyCode::Char('x'), alt), false, b"\x1bx"),
            (key(KeyCode::Enter, none), false, b"\r"),
            (key(KeyCode::Enter, KeyModifiers::SHIFT), false, b"\x1b\r"),
            (key(KeyCode::Backspace, none), false, b"\x7f"),
            (key(KeyCode::BackTab, KeyModifiers::SHIFT), false, b"\x1b[Z"),
            (key(KeyCode::Up, none), false, b"\x1b[A"),
            (key(KeyCode::Up, none), true, b"\x1bOA"),
            (key(KeyCode::Left, ctrl), true, b"\x1b[1;5D"),
            (key(KeyCode::Home, none), false, b"\x1b[H"),
            (key(KeyCode::Delete, none), false, b"\x1b[3~"),
            (
                key(KeyCode::PageUp, KeyModifiers::SHIFT),
                false,
                b"\x1b[5;2~",
            ),
            (key(KeyCode::F(1), none), false, b"\x1bOP"),
            (key(KeyCode::F(5), none), false, b"\x1b[15~"),
        ];
        for (event, application_cursor, expected) in cases {
            assert_eq!(
                encode(event, application_cursor),
                expected,
                "{event:?} (application cursor: {application_cursor})"
            );
        }
    }
}
