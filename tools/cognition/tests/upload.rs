use reqwest::{
    multipart::{Form, Part},
    Client,
};
use serde::Deserialize;
use std::{io::Write, path::PathBuf};
use zip::{
    write::{ExtendedFileOptions, FileOptions},
    ZipWriter,
};

const SERVER_URL: &str = "http://localhost:5766";

#[derive(Debug, Deserialize)]
struct Uploaded {
    message: String,
    paths: Vec<PathBuf>,
    size: usize,
}

// Helper function to create a test zip file
fn create_test_zip(files: &[(&str, &[u8])]) -> Vec<u8> {
    let mut zip_buffer = Vec::new();
    let mut zip = ZipWriter::new(std::io::Cursor::new(&mut zip_buffer));

    for (name, content) in files {
        let options: FileOptions<ExtendedFileOptions> =
            FileOptions::default().compression_method(zip::CompressionMethod::Stored).unix_permissions(0o755);
        zip.start_file(name, options).unwrap();
        zip.write_all(content).unwrap();
    }
    zip.finish().unwrap();
    zip_buffer
}

#[tokio::test]
async fn test_upload_single_file() {
    let client = Client::new();
    let content = b"Hello, World!".to_vec();
    let test_path = "test_upload/test.txt";

    // Create multipart form
    let form = Form::new()
        .part("file", Part::bytes(content.clone()).file_name("test.txt"))
        .part("metadata", Part::text(format!(r#"{{"path":"{}"}}"#, test_path)).mime_str("application/json").unwrap());

    let resp = client.post(format!("{}/upload", SERVER_URL)).multipart(form).send().await.unwrap();

    assert!(resp.status().is_success());

    // Parse and verify response
    let upload_result: Uploaded = resp.json().await.unwrap();
    assert_eq!(upload_result.message, "File uploaded successfully");
    assert_eq!(upload_result.paths, vec![PathBuf::from(test_path)]);
    assert_eq!(upload_result.size, content.len());
}

#[tokio::test]
async fn test_upload_zip_file() {
    let client = Client::new();

    // Create test zip with multiple files
    let files = vec![("file1.txt", b"Content 1" as &[u8]), ("subfolder/file2.txt", b"Content 2")];
    let zip_content = create_test_zip(&files);
    let test_path = "test_zip.zip";

    // Create multipart form
    let form = Form::new()
        .part("file", Part::bytes(zip_content.clone()).file_name("test.zip"))
        .part("metadata", Part::text(format!(r#"{{"path":"{}"}}"#, test_path)).mime_str("application/json").unwrap());

    let resp = client.post(format!("{}/upload", SERVER_URL)).multipart(form).send().await.unwrap();

    assert!(resp.status().is_success());

    // Parse and verify response
    let upload_result: Uploaded = resp.json().await.unwrap();
    assert!(upload_result.message.contains("Zip file extracted"));
    assert!(upload_result.message.contains("files successfully"));

    // Verify all expected paths are present
    let expected_paths = vec![PathBuf::from("test_zip/file1.txt"), PathBuf::from("test_zip/subfolder/file2.txt")];
    for path in expected_paths {
        assert!(upload_result.paths.contains(&path), "Missing path: {:?}", path);
    }
}

#[tokio::test]
async fn test_upload_invalid_path() {
    let client = Client::new();
    let content = b"test content".to_vec();

    // Create multipart form with invalid path
    let form = Form::new()
        .part("file", Part::bytes(content).file_name("test.txt"))
        .part("metadata", Part::text(r#"{"path":"../invalid_path"}"#).mime_str("application/json").unwrap());

    let resp = client.post(format!("{}/upload", SERVER_URL)).multipart(form).send().await.unwrap();

    // TODO: This test is failing. It's saving the file regardless of the path. We should only save the file relative to the store dir.
    assert!(resp.status().is_client_error());
}

#[tokio::test]
async fn test_upload_empty_zip() {
    let client = Client::new();

    // Create empty zip
    let zip_content = create_test_zip(&[]);
    let test_path = "empty_zip.zip";

    // Create multipart form
    let form = Form::new()
        .part("file", Part::bytes(zip_content).file_name("empty.zip"))
        .part("metadata", Part::text(format!(r#"{{"path":"{}"}}"#, test_path)).mime_str("application/json").unwrap());

    let resp = client.post(format!("{}/upload", SERVER_URL)).multipart(form).send().await.unwrap();

    assert!(resp.status().is_success());

    // Parse and verify response
    let upload_result: Uploaded = resp.json().await.unwrap();
    assert!(upload_result.message.contains("Zip file extracted"));
    assert_eq!(upload_result.paths.len(), 0);
}

#[tokio::test]
async fn test_upload_large_file() {
    let client = Client::new();

    // Create a 5MB file
    let content = vec![0u8; 5 * 1024 * 1024];
    let test_path = "large_file/large.txt";

    // Create multipart form
    let form = Form::new()
        .part("file", Part::bytes(content.clone()).file_name("large.txt"))
        .part("metadata", Part::text(format!(r#"{{"path":"{}"}}"#, test_path)).mime_str("application/json").unwrap());

    let resp = client.post(format!("{}/upload", SERVER_URL)).multipart(form).send().await.unwrap();

    assert!(resp.status().is_success());

    // Parse and verify response
    let upload_result: Uploaded = resp.json().await.unwrap();
    assert_eq!(upload_result.message, "File uploaded successfully");
    assert_eq!(upload_result.paths, vec![PathBuf::from(test_path)]);
    assert_eq!(upload_result.size, content.len());
}

#[tokio::test]
async fn test_upload_with_spaces() {
    let client = Client::new();
    let content = b"Hello, World!".to_vec();
    let test_path = "test folder/subfolder/test file.txt";

    // Create multipart form with spaces in filename and path
    let form = Form::new()
        .part("file", Part::bytes(content.clone()).file_name("test file.txt"))
        .part("metadata", Part::text(format!(r#"{{"path":"{}"}}"#, test_path)).mime_str("application/json").unwrap());

    let resp = client.post(format!("{}/upload", SERVER_URL)).multipart(form).send().await.unwrap();

    assert!(resp.status().is_success());

    // Parse and verify response
    let upload_result: Uploaded = resp.json().await.unwrap();
    assert_eq!(upload_result.message, "File uploaded successfully");
    assert_eq!(upload_result.paths, vec![PathBuf::from(test_path)]);
    assert_eq!(upload_result.size, content.len());
}
