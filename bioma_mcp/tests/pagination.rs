use bioma_mcp::server::Pagination;

#[test]
fn test_pagination_basic() {
    let pagination = Pagination::default();
    assert_eq!(pagination.size, 20);

    let custom_pagination = Pagination::new(10);
    assert_eq!(custom_pagination.size, 10);
}

#[test]
fn test_paginate_empty_collection() {
    let pagination = Pagination::new(5);
    let items: Vec<i32> = vec![];

    let (results, next_cursor) = pagination.paginate(&items, None, |&item| item);

    assert!(results.is_empty());
    assert_eq!(next_cursor, None);
}

#[test]
fn test_paginate_complete_flow() {
    let pagination = Pagination::new(3);
    let items: Vec<i32> = (1..=8).collect();

    let (results, next_cursor) = pagination.paginate(&items, None, |&item| item);
    assert_eq!(results, vec![1, 2, 3]);
    assert!(next_cursor.is_some());

    let (results, next_cursor) = pagination.paginate(&items, next_cursor, |&item| item);
    assert_eq!(results, vec![4, 5, 6]);
    assert!(next_cursor.is_some());

    let (results, next_cursor) = pagination.paginate(&items, next_cursor, |&item| item);
    assert_eq!(results, vec![7, 8]);
    assert_eq!(next_cursor, None);
}

#[test]
fn test_data_conversion() {
    struct TestItem {
        name: String,
    }

    let pagination = Pagination::new(2);
    let items = vec![
        TestItem { name: "one".to_string() },
        TestItem { name: "two".to_string() },
        TestItem { name: "three".to_string() },
    ];

    let (results, _) = pagination.paginate(&items, None, |item| item.name.clone());

    assert_eq!(results, vec!["one", "two"]);
}

#[test]
fn test_invalid_cursor_handling() {
    let pagination = Pagination::new(5);
    let items: Vec<i32> = (1..=10).collect();

    let (results, _) = pagination.paginate(&items, Some("not_valid".to_string()), |&item| item);
    assert_eq!(results, vec![1, 2, 3, 4, 5]);

    let cursor = pagination.encode_cursor(20);
    let (results, next_cursor) = pagination.paginate(&items, Some(cursor), |&item| item);
    assert!(results.is_empty());
    assert_eq!(next_cursor, None);
}

#[test]
fn test_cursor_encoding() {
    let pagination = Pagination::new(5);
    let offset = 42;

    let encoded = pagination.encode_cursor(offset);
    let decoded = pagination.decode_cursor(&encoded).unwrap();

    assert_eq!(decoded, offset);

    assert!(pagination.decode_cursor("invalid base64!").is_err());
}

#[test]
fn test_collection_changes() {
    let pagination = Pagination::new(3);
    let mut items = vec![1, 2, 3, 4, 5];

    let (_, next_cursor) = pagination.paginate(&items, None, |&item| item);

    items.push(6);
    items.push(7);

    let (results, _) = pagination.paginate(&items, next_cursor, |&item| item);
    assert_eq!(results, vec![4, 5, 6]);
}
