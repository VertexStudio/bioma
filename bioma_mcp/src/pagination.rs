use base64;
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct CursorInfo {
    pub page: usize,
}

#[derive(Clone, Debug)]
pub struct PaginationConfig {
    pub page_size: usize,
}

impl Default for PaginationConfig {
    fn default() -> Self {
        Self { page_size: 10 }
    }
}

pub mod cursor {
    use super::*;

    pub fn encode(info: &CursorInfo) -> String {
        let json = serde_json::to_string(info).unwrap_or_default();
        base64::encode(json)
    }

    pub fn decode(cursor: &str) -> Option<CursorInfo> {
        let bytes = match base64::decode(cursor) {
            Ok(bytes) => bytes,
            Err(_) => return None,
        };

        let json = match String::from_utf8(bytes) {
            Ok(json) => json,
            Err(_) => return None,
        };

        serde_json::from_str(&json).ok()
    }
}

pub fn paginate<T: Clone>(items: &[T], cursor: Option<&str>, config: &PaginationConfig) -> (Vec<T>, Option<String>) {
    let current_page = match cursor {
        Some(cursor_str) => cursor::decode(cursor_str).map(|info| info.page).unwrap_or(0),
        None => 0,
    };

    let start_idx = current_page * config.page_size;
    let end_idx = std::cmp::min(start_idx + config.page_size, items.len());

    if start_idx >= items.len() {
        return (Vec::new(), None);
    }

    let items_slice = &items[start_idx..end_idx];

    let next_cursor =
        if end_idx < items.len() { Some(cursor::encode(&CursorInfo { page: current_page + 1 })) } else { None };

    (items_slice.to_vec(), next_cursor)
}

pub fn validate_cursor(cursor: Option<&str>) -> bool {
    match cursor {
        Some(cursor_str) => cursor::decode(cursor_str).is_some(),
        None => true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cursor_encoding_decoding() {
        let info = CursorInfo { page: 5 };
        let encoded = cursor::encode(&info);
        let decoded = cursor::decode(&encoded).unwrap();
        assert_eq!(decoded.page, 5);
    }

    #[test]
    fn test_pagination_first_page() {
        let items = vec![1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12];
        let config = PaginationConfig { page_size: 5 };
        let (page, next_cursor) = paginate(&items, None, &config);

        assert_eq!(page, vec![1, 2, 3, 4, 5]);
        assert!(next_cursor.is_some());
    }

    #[test]
    fn test_pagination_middle_page() {
        let items = vec![1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12];
        let cursor = cursor::encode(&CursorInfo { page: 1 });
        let config = PaginationConfig { page_size: 5 };
        let (page, next_cursor) = paginate(&items, Some(&cursor), &config);

        assert_eq!(page, vec![6, 7, 8, 9, 10]);
        assert!(next_cursor.is_some());
    }

    #[test]
    fn test_pagination_last_page() {
        let items = vec![1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12];
        let cursor = cursor::encode(&CursorInfo { page: 2 });
        let config = PaginationConfig { page_size: 5 };
        let (page, next_cursor) = paginate(&items, Some(&cursor), &config);

        assert_eq!(page, vec![11, 12]);
        assert!(next_cursor.is_none());
    }

    #[test]
    fn test_validate_cursor() {
        let valid_cursor = cursor::encode(&CursorInfo { page: 1 });
        assert!(validate_cursor(Some(&valid_cursor)));
        assert!(validate_cursor(None));
        assert!(!validate_cursor(Some("invalid-cursor")));
    }
}
