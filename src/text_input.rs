pub fn char_len(value: &str) -> usize {
    value.chars().count()
}

pub fn byte_index(value: &str, char_index: usize) -> usize {
    value
        .char_indices()
        .nth(char_index)
        .map(|(idx, _)| idx)
        .unwrap_or(value.len())
}

/// Insert `ch` at char position `cursor`, return new cursor.
pub fn insert_at(value: &mut String, cursor: usize, ch: char) -> usize {
    let idx = byte_index(value, cursor);
    value.insert(idx, ch);
    cursor + 1
}

/// Delete char before `cursor`, return new cursor.
pub fn backspace_at(value: &mut String, cursor: usize) -> usize {
    if cursor == 0 {
        return 0;
    }
    let new_cursor = cursor - 1;
    let idx = byte_index(value, new_cursor);
    value.remove(idx);
    new_cursor
}

/// Move the cursor one char left (saturating at 0).
pub fn move_left(cursor: usize) -> usize {
    cursor.saturating_sub(1)
}

/// Move the cursor one char right (clamped to end of `value`).
pub fn move_right(value: &str, cursor: usize) -> usize {
    (cursor + 1).min(char_len(value))
}

/// Delete the char AT `cursor` (forward delete). Cursor is unchanged; a no-op
/// at end of string.
pub fn delete_at(value: &mut String, cursor: usize) -> usize {
    if cursor >= char_len(value) {
        return cursor;
    }
    let idx = byte_index(value, cursor);
    value.remove(idx);
    cursor
}

/// Kill everything before `cursor` (readline's `unix-line-discard`, Ctrl+U).
/// With the cursor parked at the end of a field this wipes the whole value in
/// one chord instead of one backspace per character. Returns the new cursor.
pub fn clear_before_cursor(value: &mut String, cursor: usize) -> usize {
    let clamped = cursor.min(char_len(value));
    let idx = byte_index(value, clamped);
    value.replace_range(..idx, "");
    0
}

/// Delete the word before `cursor` (readline's `backward-kill-word`, Ctrl+W). A
/// word is a run of non-space characters; the spaces *after* it go with it, the
/// one in front stays, so repeated chords keep eating back through a value one
/// word at a time. Returns the new cursor.
pub fn delete_word_before(value: &mut String, cursor: usize) -> usize {
    let len = char_len(value);
    let mut pos = cursor.min(len);
    while pos > 0 && value[..byte_index(value, pos)].ends_with(' ') {
        pos -= 1;
    }
    while pos > 0 {
        let prev = byte_index(value, pos - 1);
        if value.as_bytes()[prev] == b' ' {
            break;
        }
        pos -= 1;
    }
    let idx = byte_index(value, pos);
    value.replace_range(idx..byte_index(value, cursor.min(len)), "");
    pos
}

/// Handle a cursor-movement / forward-delete key on a `(value, cursor)` pair —
/// the shared body of every form's Left/Right/Home/End/Delete handling.
/// Returns `None` when the key isn't one of those; `Some(changed)` otherwise,
/// where `changed` is true only when Delete actually removed a character (so
/// callers can set their dirty flag).
pub fn handle_cursor_key(
    code: crossterm::event::KeyCode,
    value: &mut String,
    cursor: &mut usize,
) -> Option<bool> {
    use crossterm::event::KeyCode;
    match code {
        KeyCode::Left => {
            *cursor = move_left(*cursor);
            Some(false)
        }
        KeyCode::Right => {
            *cursor = move_right(value, *cursor);
            Some(false)
        }
        KeyCode::Home => {
            *cursor = 0;
            Some(false)
        }
        KeyCode::End => {
            *cursor = char_len(value);
            Some(false)
        }
        KeyCode::Delete => {
            let before = char_len(value);
            *cursor = delete_at(value, *cursor);
            Some(char_len(value) != before)
        }
        _ => None,
    }
}

pub fn with_cursor(value: &str, cursor: usize) -> String {
    if value.is_empty() {
        return "_".to_string();
    }
    let idx = byte_index(value, cursor.min(char_len(value)));
    let (before, after) = value.split_at(idx);
    format!("{before}_{after}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn insert_handles_multibyte_at_start_middle_and_end() {
        let mut value = "првет".to_string();
        let cursor = insert_at(&mut value, 2, 'и');
        assert_eq!(value, "привет");
        assert_eq!(cursor, 3);

        let cursor = insert_at(&mut value, 0, '🙂');
        assert_eq!(value, "🙂привет");
        assert_eq!(cursor, 1);

        let len = char_len(&value);
        let cursor = insert_at(&mut value, len, '!');
        assert_eq!(value, "🙂привет!");
        assert_eq!(cursor, 8);
    }

    #[test]
    fn backspace_handles_multibyte_at_start_middle_and_end() {
        let mut value = "🙂привет!".to_string();
        let cursor = backspace_at(&mut value, 1);
        assert_eq!(value, "привет!");
        assert_eq!(cursor, 0);

        let cursor = backspace_at(&mut value, 3);
        assert_eq!(value, "првет!");
        assert_eq!(cursor, 2);

        let len = char_len(&value);
        let cursor = backspace_at(&mut value, len);
        assert_eq!(value, "првет");
        assert_eq!(cursor, 5);
    }

    #[test]
    fn backspace_at_zero_is_noop() {
        let mut value = "abc".to_string();
        let cursor = backspace_at(&mut value, 0);
        assert_eq!(value, "abc");
        assert_eq!(cursor, 0);
    }

    #[test]
    fn with_cursor_splits_on_char_boundary() {
        assert_eq!(with_cursor("абв", 1), "а_бв");
        assert_eq!(with_cursor("a🙂b", 2), "a🙂_b");
        assert_eq!(with_cursor("", 0), "_");
    }

    #[test]
    fn move_left_right_clamp() {
        assert_eq!(move_left(0), 0);
        assert_eq!(move_left(3), 2);
        assert_eq!(move_right("абв", 1), 2);
        assert_eq!(move_right("абв", 3), 3); // clamped at end
        assert_eq!(move_right("", 0), 0);
    }

    #[test]
    fn delete_at_removes_char_at_cursor() {
        let mut v = "привет".to_string();
        let c = delete_at(&mut v, 0); // delete 'п'
        assert_eq!(v, "ривет");
        assert_eq!(c, 0);

        let c = delete_at(&mut v, 2); // delete 'в' (multibyte-safe)
        assert_eq!(v, "риет");
        assert_eq!(c, 2);

        // At end of string: no-op.
        let len = char_len(&v);
        let c = delete_at(&mut v, len);
        assert_eq!(v, "риет");
        assert_eq!(c, len);
    }

    #[test]
    fn clear_before_cursor_wipes_the_whole_field_from_the_end() {
        let mut v = "s3cret-pw".to_string();
        let len = char_len(&v);
        let c = clear_before_cursor(&mut v, len);
        assert_eq!(v, "");
        assert_eq!(c, 0);
    }

    #[test]
    fn clear_before_cursor_keeps_the_tail() {
        let mut v = "user@host".to_string();
        let c = clear_before_cursor(&mut v, 4); // kill "user"
        assert_eq!(v, "@host");
        assert_eq!(c, 0);
    }

    #[test]
    fn clear_before_cursor_is_multibyte_safe_and_clamps() {
        let mut v = "привет".to_string();
        // Cursor past the end clamps instead of panicking on a byte boundary.
        let c = clear_before_cursor(&mut v, 99);
        assert_eq!(v, "");
        assert_eq!(c, 0);

        let mut v = "🙂🙂".to_string();
        let _ = clear_before_cursor(&mut v, 1); // kill one emoji
        assert_eq!(v, "🙂");
    }

    #[test]
    fn delete_word_before_eats_one_word_plus_trailing_space() {
        let mut v = "docker compose up".to_string();
        let end = char_len(&v);
        let c = delete_word_before(&mut v, end);
        assert_eq!(v, "docker compose ");
        assert_eq!(c, char_len("docker compose "));

        // Second chord eats the previous word too.
        assert_eq!(delete_word_before(&mut v, c), char_len("docker "));
        assert_eq!(v, "docker ");
    }

    #[test]
    fn delete_word_before_stops_at_start_and_handles_empty_gap() {
        let mut v = String::from("word");
        let end = char_len(&v);
        let c = delete_word_before(&mut v, end);
        assert_eq!(v, "");
        assert_eq!(c, 0);

        // Nothing to delete: no-op, cursor unchanged.
        let mut v = String::new();
        let c = delete_word_before(&mut v, 0);
        assert_eq!(v, "");
        assert_eq!(c, 0);
    }
}
