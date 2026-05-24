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
}
