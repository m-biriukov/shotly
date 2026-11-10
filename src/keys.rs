//! Keyboard shortcuts that work in any keyboard layout.
//!
//! The same physical key types different characters in different layouts
//! (the key labelled R types "к" on a Russian or Ukrainian layout), so
//! shortcuts cannot just compare the typed character.

use winit::keyboard::{Key, KeyCode, PhysicalKey};

/// The Latin letter or digit a shortcut key press stands for, lower-case.
///
/// If the active layout types a Latin letter, that wins, so Dvorak, Colemak
/// and the like keep the letters printed on their keys. Otherwise (Cyrillic,
/// Greek, Hebrew, …) it is the letter at that key's position on a US
/// keyboard, the way macOS treats ⌘-shortcuts.
pub fn latin_shortcut(logical: &Key, physical: PhysicalKey) -> Option<char> {
    if let Key::Character(s) = logical {
        let mut chars = s.chars();
        if let (Some(c), None) = (chars.next(), chars.next())
            && c.is_ascii_alphanumeric()
        {
            return Some(c.to_ascii_lowercase());
        }
    }
    match physical {
        PhysicalKey::Code(code) => letter_at(code),
        PhysicalKey::Unidentified(_) => None,
    }
}

/// Letter printed on the key at this position of a US keyboard.
fn letter_at(code: KeyCode) -> Option<char> {
    Some(match code {
        KeyCode::KeyA => 'a',
        KeyCode::KeyB => 'b',
        KeyCode::KeyC => 'c',
        KeyCode::KeyD => 'd',
        KeyCode::KeyE => 'e',
        KeyCode::KeyF => 'f',
        KeyCode::KeyG => 'g',
        KeyCode::KeyH => 'h',
        KeyCode::KeyI => 'i',
        KeyCode::KeyJ => 'j',
        KeyCode::KeyK => 'k',
        KeyCode::KeyL => 'l',
        KeyCode::KeyM => 'm',
        KeyCode::KeyN => 'n',
        KeyCode::KeyO => 'o',
        KeyCode::KeyP => 'p',
        KeyCode::KeyQ => 'q',
        KeyCode::KeyR => 'r',
        KeyCode::KeyS => 's',
        KeyCode::KeyT => 't',
        KeyCode::KeyU => 'u',
        KeyCode::KeyV => 'v',
        KeyCode::KeyW => 'w',
        KeyCode::KeyX => 'x',
        KeyCode::KeyY => 'y',
        KeyCode::KeyZ => 'z',
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use winit::keyboard::{NamedKey, NativeKeyCode, SmolStr};

    fn typed(s: &str) -> Key {
        Key::Character(SmolStr::new(s))
    }

    #[test]
    fn latin_layouts_use_the_typed_letter() {
        assert_eq!(
            latin_shortcut(&typed("r"), PhysicalKey::Code(KeyCode::KeyR)),
            Some('r')
        );
        assert_eq!(
            latin_shortcut(&typed("R"), PhysicalKey::Code(KeyCode::KeyR)),
            Some('r')
        );
        // Dvorak: the key at the C position types "j"; the letter on the keycap wins.
        assert_eq!(
            latin_shortcut(&typed("j"), PhysicalKey::Code(KeyCode::KeyC)),
            Some('j')
        );
    }

    #[test]
    fn cyrillic_layouts_fall_back_to_the_key_position() {
        // Russian/Ukrainian: R→к, O→щ, C→с, S→і, Z→я.
        for (ch, code, want) in [
            ("к", KeyCode::KeyR, 'r'),
            ("щ", KeyCode::KeyO, 'o'),
            ("с", KeyCode::KeyC, 'c'),
            ("і", KeyCode::KeyS, 's'),
            ("я", KeyCode::KeyZ, 'z'),
            ("К", KeyCode::KeyR, 'r'),
        ] {
            assert_eq!(
                latin_shortcut(&typed(ch), PhysicalKey::Code(code)),
                Some(want),
                "{ch}"
            );
        }
    }

    #[test]
    fn non_letters_and_unknown_keys_give_nothing() {
        let unknown = PhysicalKey::Unidentified(NativeKeyCode::Unidentified);
        assert_eq!(
            latin_shortcut(
                &Key::Named(NamedKey::Shift),
                PhysicalKey::Code(KeyCode::ShiftLeft)
            ),
            None
        );
        assert_eq!(latin_shortcut(&typed("к"), unknown), None);
        assert_eq!(
            latin_shortcut(&typed("."), PhysicalKey::Code(KeyCode::Period)),
            None
        );
        // Option+letter types a symbol; the key position still identifies it.
        assert_eq!(
            latin_shortcut(&typed("å"), PhysicalKey::Code(KeyCode::KeyA)),
            Some('a')
        );
    }
}
