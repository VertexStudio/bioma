use base64;
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Cursor {
    pub page: usize,
}

impl Cursor {
    pub fn encode(&self) -> String {
        let json = serde_json::to_string(self).unwrap_or_default();
        base64::encode(json)
    }

    pub fn decode(cursor: &str) -> Option<Self> {
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

    pub fn validate(cursor: Option<&str>) -> bool {
        match cursor {
            Some(cursor_str) => Self::decode(cursor_str).is_some(),
            None => true,
        }
    }
}

#[derive(Clone, Debug)]
pub struct Pagination {
    pub size: usize,
}

impl Default for Pagination {
    fn default() -> Self {
        Self { size: 10 }
    }
}

impl Pagination {
    pub fn paginate<T: Clone>(&self, items: &[T], cursor: Option<&str>) -> (Vec<T>, Option<String>) {
        let current_page = match cursor {
            Some(cursor_str) => Cursor::decode(cursor_str).map(|info| info.page).unwrap_or(0),
            None => 0,
        };

        let start_idx = current_page * self.size;
        let end_idx = std::cmp::min(start_idx + self.size, items.len());

        if start_idx >= items.len() {
            return (Vec::new(), None);
        }

        let items_slice = &items[start_idx..end_idx];

        let next_cursor = if end_idx < items.len() { Some(Cursor { page: current_page + 1 }.encode()) } else { None };

        (items_slice.to_vec(), next_cursor)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cursor_encoding_decoding() {
        let info = Cursor { page: 5 };
        let encoded = info.encode();
        let decoded = Cursor::decode(&encoded).unwrap();
        assert_eq!(decoded.page, 5);
    }

    #[test]
    fn test_pagination_first_page() {
        let items = vec![1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12];
        let config = Pagination { size: 5 };
        let (page, next_cursor) = config.paginate(&items, None);

        assert_eq!(page, vec![1, 2, 3, 4, 5]);
        assert!(next_cursor.is_some());
    }

    #[test]
    fn test_pagination_middle_page() {
        let items = vec![1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12];
        let cursor = Cursor { page: 1 }.encode();
        let config = Pagination { size: 5 };
        let (page, next_cursor) = config.paginate(&items, Some(&cursor));

        assert_eq!(page, vec![6, 7, 8, 9, 10]);
        assert!(next_cursor.is_some());
    }

    #[test]
    fn test_pagination_last_page() {
        let items = vec![1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12];
        let cursor = Cursor { page: 2 }.encode();
        let config = Pagination { size: 5 };
        let (page, next_cursor) = config.paginate(&items, Some(&cursor));

        assert_eq!(page, vec![11, 12]);
        assert!(next_cursor.is_none());
    }

    #[test]
    fn test_validate_cursor() {
        let valid_cursor = Cursor { page: 1 }.encode();
        assert!(Cursor::validate(Some(&valid_cursor)));
        assert!(Cursor::validate(None));
        assert!(!Cursor::validate(Some("invalid-cursor")));
    }
}
