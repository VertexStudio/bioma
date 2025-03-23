use crate::resources::{ResourceDef, ResourceError};
use crate::schema::{ReadResourceResult, ResourceTemplate};
use notify::{Config, Event, RecommendedWatcher, RecursiveMode, Watcher};
use serde::Serialize;
use std::any::Any;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::fs;
use tokio::io::AsyncReadExt;
use tokio::sync::mpsc;
use tokio::sync::Mutex;
use tokio::task::JoinHandle;
use tracing::info;
use url::Url;

/// A file system resource that can serve both text and binary files
#[derive(Clone, Serialize)]
pub struct FileSystem {
    /// Base directory for file accesses
    #[serde(skip)]
    base_dir: Arc<PathBuf>,
    /// Active watchers per resource URI
    #[serde(skip)]
    watchers: Arc<Mutex<HashMap<String, JoinHandle<()>>>>,
}

impl FileSystem {
    /// Create a new FileSystem resource with the given base directory
    pub fn new<P: AsRef<Path>>(base_dir: P) -> Self {
        Self { base_dir: Arc::new(base_dir.as_ref().to_path_buf()), watchers: Arc::new(Mutex::new(HashMap::new())) }
    }

    async fn watch_path(path: impl AsRef<Path>, tx: mpsc::Sender<()>) -> Result<JoinHandle<()>, ResourceError> {
        let path = path.as_ref().to_owned();

        let handle = tokio::spawn(async move {
            // Create a channel for the watcher to send notifications to this task
            let (watcher_tx, mut watcher_rx) = mpsc::channel(16);

            // Create the watcher
            let mut watcher = RecommendedWatcher::new(
                move |res: notify::Result<Event>| {
                    if res.is_ok() {
                        let _ = watcher_tx.blocking_send(());
                    }
                },
                Config::default(),
            )
            .unwrap();

            // Start watching the path
            watcher.watch(&path, RecursiveMode::Recursive).unwrap();

            // Forward notifications until the sender channel is closed
            while watcher_rx.recv().await.is_some() {
                if tx.send(()).await.is_err() {
                    // Receiver dropped, exit the task
                    break;
                }
            }
        });

        Ok(handle)
    }

    /// Get a file's MIME type based on its extension
    fn guess_mime_type(path: &Path) -> Option<String> {
        match path.extension().and_then(|e| e.to_str()) {
            Some("txt") => Some("text/plain".to_string()),
            Some("md") => Some("text/markdown".to_string()),
            Some("html") => Some("text/html".to_string()),
            Some("css") => Some("text/css".to_string()),
            Some("js") => Some("application/javascript".to_string()),
            Some("json") => Some("application/json".to_string()),
            Some("xml") => Some("application/xml".to_string()),
            Some("pdf") => Some("application/pdf".to_string()),
            Some("png") => Some("image/png".to_string()),
            Some("jpg") | Some("jpeg") => Some("image/jpeg".to_string()),
            Some("gif") => Some("image/gif".to_string()),
            Some("svg") => Some("image/svg+xml".to_string()),
            _ => None,
        }
    }

    /// Determine if a file should be treated as binary based on its MIME type
    fn is_binary(mime_type: &str) -> bool {
        !mime_type.starts_with("text/")
            && !mime_type.contains("json")
            && !mime_type.contains("xml")
            && !mime_type.contains("javascript")
    }

    /// Convert a URI to a file path
    fn uri_to_path(&self, uri: &str) -> Result<PathBuf, ResourceError> {
        let url = Url::parse(uri).map_err(|e| ResourceError::Custom(format!("Invalid URI: {}", e)))?;

        // Ensure URI uses the file scheme
        if url.scheme() != "file" {
            return Err(ResourceError::Custom(format!("Unsupported URI scheme: {}", url.scheme())));
        }

        // Extract the path component
        let path_str = url.path();

        // Use base directory for empty paths (root URIs)
        if path_str.is_empty() || path_str == "/" {
            return Ok((*self.base_dir).clone());
        }

        let path = Path::new(path_str);

        // Ensure the path is within the base directory
        let absolute_path = if path.is_absolute() { path.to_path_buf() } else { self.base_dir.join(path) };

        // For paths that don't exist yet or special paths, skip canonicalization
        if !absolute_path.exists() {
            return Err(ResourceError::NotFound(format!("File not found: {}", absolute_path.display())));
        }

        // Use canonical paths to prevent directory traversal attacks
        let canonical_base = self
            .base_dir
            .canonicalize()
            .map_err(|e| ResourceError::Custom(format!("Failed to canonicalize base directory: {}", e)))?;

        let canonical_path = absolute_path.canonicalize().map_err(|e| {
            // If the file doesn't exist, provide a clearer error
            if e.kind() == std::io::ErrorKind::NotFound {
                return ResourceError::NotFound(format!("File not found: {}", absolute_path.display()));
            }
            ResourceError::Custom(format!("Failed to canonicalize path: {}", e))
        })?;

        // Check if the path is within the base directory or is the base directory itself
        if !canonical_path.starts_with(&canonical_base) && canonical_path != canonical_base {
            return Err(ResourceError::Custom(format!(
                "Access denied: {} is outside the base directory",
                absolute_path.display()
            )));
        }

        Ok(canonical_path)
    }

    /// Convert a file path to a URI
    fn path_to_uri(&self, path: &Path) -> String {
        let path_str = path.to_string_lossy();
        format!("file://{}", path_str)
    }
}

impl ResourceDef for FileSystem {
    const NAME: &'static str = "filesystem";
    const DESCRIPTION: &'static str = "Access files from the server's filesystem";
    const URI: &'static str = "file:///";
    const MIME_TYPE: Option<&'static str> = None; // Varies by file

    fn template() -> Option<ResourceTemplate> {
        Some(ResourceTemplate {
            name: Self::NAME.to_string(),
            description: Some(Self::DESCRIPTION.to_string()),
            uri_template: "file:///{path}".to_string(),
            mime_type: None, // Varies by file
            annotations: None,
        })
    }

    async fn read(&self, uri: String) -> Result<ReadResourceResult, ResourceError> {
        // Convert URI to file path
        let path = self.uri_to_path(&uri)?;

        // Get file metadata
        let metadata = fs::metadata(&path).await.map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                ResourceError::NotFound(format!("File not found: {}", path.display()))
            } else {
                ResourceError::Reading(format!("Failed to read file metadata: {}", e))
            }
        })?;

        if metadata.is_dir() {
            // If it's a directory, list its contents
            let mut entries = fs::read_dir(&path)
                .await
                .map_err(|e| ResourceError::Reading(format!("Failed to read directory: {}", e)))?;

            let mut contents = Vec::new();

            while let Some(entry) = entries
                .next_entry()
                .await
                .map_err(|e| ResourceError::Reading(format!("Failed to read directory entry: {}", e)))?
            {
                let entry_path = entry.path();
                let entry_uri = self.path_to_uri(&entry_path);
                let entry_name = entry_path.file_name().unwrap_or_default().to_string_lossy().to_string();
                let entry_metadata = entry
                    .metadata()
                    .await
                    .map_err(|e| ResourceError::Reading(format!("Failed to read entry metadata: {}", e)))?;

                let is_dir = entry_metadata.is_dir();
                let mime_type =
                    if is_dir { Some("inode/directory".to_string()) } else { Self::guess_mime_type(&entry_path) };

                contents.push(serde_json::json!({
                    "uri": entry_uri,
                    "mimeType": mime_type,
                    "text": format!("{}{}", entry_name, if is_dir { "/" } else { "" }),
                }));
            }

            Ok(ReadResourceResult { contents, meta: None })
        } else {
            // It's a file, read its contents
            let mime_type = Self::guess_mime_type(&path);
            let is_binary = mime_type.as_ref().map_or(false, |mt| Self::is_binary(mt));

            if is_binary {
                // Read as binary data
                let mut file = fs::File::open(&path)
                    .await
                    .map_err(|e| ResourceError::Reading(format!("Failed to open file: {}", e)))?;

                let mut buffer = Vec::new();
                file.read_to_end(&mut buffer)
                    .await
                    .map_err(|e| ResourceError::Reading(format!("Failed to read file: {}", e)))?;

                // Encode as base64
                let base64_data = base64::encode(&buffer);

                Ok(ReadResourceResult {
                    contents: vec![serde_json::json!({
                        "uri": uri,
                        "mimeType": mime_type,
                        "blob": base64_data,
                    })],
                    meta: None,
                })
            } else {
                // Read as text
                let content = fs::read_to_string(&path)
                    .await
                    .map_err(|e| ResourceError::Reading(format!("Failed to read file: {}", e)))?;

                Ok(ReadResourceResult {
                    contents: vec![serde_json::json!({
                        "uri": uri,
                        "mimeType": mime_type,
                        "text": content,
                    })],
                    meta: None,
                })
            }
        }
    }

    fn supports_subscription() -> bool {
        true
    }

    async fn subscribe(&self, uri: String, on_resource_updated_tx: mpsc::Sender<()>) -> Result<(), ResourceError> {
        let path = self.uri_to_path(&uri)?;
        if !path.exists() {
            return Err(ResourceError::NotFound(format!("Cannot subscribe to non-existent resource: {}", uri)));
        }
        let handle = FileSystem::watch_path(path, on_resource_updated_tx).await?;
        self.watchers.lock().await.insert(uri.clone(), handle);
        info!("Subscribed to resource: {}", uri);
        Ok(())
    }

    async fn unsubscribe(&self, uri: String) -> Result<(), ResourceError> {
        self.watchers.lock().await.remove(&uri);
        info!("Unsubscribed from resource: {}", uri);
        Ok(())
    }
}

// Add as_any method implementation for the trait
impl AsRef<dyn Any> for FileSystem {
    fn as_ref(&self) -> &(dyn Any + 'static) {
        self
    }
}

#[cfg(test)]
mod tests {

    use super::*;
    use std::fs::File;
    use std::io::Write;
    use tempfile::tempdir;

    #[tokio::test]
    async fn test_filesystem_read_text() {
        // Create a temporary directory
        let temp_dir = tempdir().unwrap();
        let file_path = temp_dir.path().join("test.txt");

        // Write a test file
        {
            let mut file = File::create(&file_path).unwrap();
            write!(file, "Hello, world!").unwrap();
        }

        // Create the resource
        let fs_resource = FileSystem::new(temp_dir.path());

        // Read the file
        let uri = format!("file://{}", file_path.display());
        let result = fs_resource.read(uri).await.unwrap();

        // Verify the content
        let content = &result.contents[0];
        let text = content["text"].as_str().expect("text field should be a string");
        assert_eq!(text, "Hello, world!");
    }

    #[tokio::test]
    async fn test_filesystem_not_found() {
        // Create a temporary directory
        let temp_dir = tempdir().unwrap();

        // Create the resource
        let fs_resource = FileSystem::new(temp_dir.path());

        // Try to read a nonexistent file
        let uri = format!("file://{}/nonexistent.txt", temp_dir.path().display());
        let result = fs_resource.read(uri).await;

        // Verify the error
        assert!(result.is_err());
        match result {
            Err(ResourceError::NotFound(_)) => {}
            _ => panic!("Expected NotFound error"),
        }
    }

    #[tokio::test]
    async fn test_filesystem_read_directory() {
        // Create a temporary directory
        let temp_dir = tempdir().unwrap();

        // Create some test files and subdirectories
        let file1_path = temp_dir.path().join("test1.txt");
        let file2_path = temp_dir.path().join("test2.txt");
        let subdir_path = temp_dir.path().join("subdir");

        {
            let mut file1 = File::create(&file1_path).unwrap();
            write!(file1, "File 1 content").unwrap();

            let mut file2 = File::create(&file2_path).unwrap();
            write!(file2, "File 2 content").unwrap();

            std::fs::create_dir(&subdir_path).unwrap();
        }

        // Create the resource
        let fs_resource = FileSystem::new(temp_dir.path());

        // Read the directory
        let uri = format!("file://{}", temp_dir.path().display());
        let result = fs_resource.read(uri).await.unwrap();

        // Verify the contents
        assert_eq!(result.contents.len(), 3); // Should have 3 entries

        // Verify each entry exists
        let entries: Vec<&str> = result.contents.iter().map(|entry| entry["text"].as_str().unwrap()).collect();

        assert!(entries.contains(&"test1.txt"));
        assert!(entries.contains(&"test2.txt"));
        assert!(entries.contains(&"subdir/"));
    }

    #[tokio::test]
    async fn test_filesystem_root_uri() {
        // Create a temporary directory
        let temp_dir = tempdir().unwrap();

        // Create some test files
        let file_path = temp_dir.path().join("test.txt");
        {
            let mut file = File::create(&file_path).unwrap();
            write!(file, "Root test").unwrap();
        }

        // Create the resource
        let fs_resource = FileSystem::new(temp_dir.path());

        // Test root URI handling
        let result = fs_resource.read("file:///".to_string()).await.unwrap();

        // Verify the contents include our test file
        let entries: Vec<&str> = result.contents.iter().map(|entry| entry["text"].as_str().unwrap()).collect();
        assert!(entries.contains(&"test.txt"));
    }

    #[tokio::test]
    async fn test_uri_to_path_root() {
        // Create a temporary directory
        let temp_dir = tempdir().unwrap();

        // Create the resource
        let fs_resource = FileSystem::new(temp_dir.path());

        // Test conversion of root URI to path
        let path = fs_resource.uri_to_path("file:///").unwrap();
        assert_eq!(path, temp_dir.path());

        // Also test with alternate root URI form
        let path2 = fs_resource.uri_to_path("file://").unwrap();
        assert_eq!(path2, temp_dir.path());
    }

    #[tokio::test]
    async fn test_path_to_uri() {
        // Create a temporary directory
        let temp_dir = tempdir().unwrap();

        // Create the resource
        let fs_resource = FileSystem::new(temp_dir.path());

        // Test conversion of path to URI
        let uri = fs_resource.path_to_uri(temp_dir.path());
        assert!(uri.starts_with("file://"));
        assert!(uri.ends_with(&*temp_dir.path().to_string_lossy()));
    }

    #[tokio::test]
    async fn test_nonexistent_directory() {
        // Create a temporary directory
        let temp_dir = tempdir().unwrap();

        // Create the resource
        let fs_resource = FileSystem::new(temp_dir.path());

        // Test reading a nonexistent directory
        let nonexistent_dir = format!("file://{}/nonexistent_dir/", temp_dir.path().display());
        let result = fs_resource.read(nonexistent_dir).await;

        // Verify we get the correct error
        assert!(result.is_err());
        match result {
            Err(ResourceError::NotFound(_)) => {}
            _ => panic!("Expected NotFound error"),
        }
    }

    #[tokio::test]
    async fn test_subscription_and_notification() {}
}
