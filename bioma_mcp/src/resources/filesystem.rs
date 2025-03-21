use crate::resources::{ResourceDef, ResourceError, ResourceManager};
use crate::schema::{ReadResourceResult, ResourceTemplate};
use crate::ClientId;
use notify::{Config, Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use serde::Serialize;
use std::any::Any;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex as StdMutex};
use tokio::fs;
use tokio::io::AsyncReadExt;
use tokio::sync::mpsc::{self, Receiver, Sender};
use tokio::sync::Mutex as TokioMutex;
use tracing::{debug, error, info};
use url::Url;

/// A file system resource that can serve both text and binary files
#[derive(Clone, Serialize)]
pub struct FileSystem {
    /// Base directory for file accesses
    #[serde(skip)]
    base_dir: Arc<PathBuf>,
    /// Cached file metadata
    #[serde(skip)]
    cache: Arc<StdMutex<HashMap<String, FileMetadata>>>,
    /// Resource manager for subscriptions
    #[serde(skip)]
    resource_manager: Arc<ResourceManager>,
    /// Active watchers per resource URI
    #[serde(skip)]
    watchers: Arc<TokioMutex<HashMap<String, WatcherHandle>>>,
}

/// Manages an active filesystem watcher instance and its subscriber count
struct WatcherHandle {
    _watcher: RecommendedWatcher, // Retained for lifetime management
    subscriber_count: usize,
}

/// Cached metadata about a file to avoid redundant filesystem operations
#[derive(Clone, Debug, Serialize)]
struct FileMetadata {
    pub path: PathBuf,
    pub mime_type: Option<String>,
    pub is_binary: bool,
    pub last_modified: std::time::SystemTime,
}

impl FileSystem {
    /// Create a new FileSystem resource with the given base directory
    pub fn new<P: AsRef<Path>>(base_dir: P) -> Self {
        Self {
            base_dir: Arc::new(base_dir.as_ref().to_path_buf()),
            cache: Arc::new(StdMutex::new(HashMap::new())),
            resource_manager: Arc::new(ResourceManager::new()),
            watchers: Arc::new(TokioMutex::new(HashMap::new())),
        }
    }

    /// Get the resource manager
    pub fn get_resource_manager(&self) -> Arc<ResourceManager> {
        self.resource_manager.clone()
    }

    /// Creates a filesystem watcher for a specific resource URI
    ///
    /// Each URI can have a single watcher with multiple subscribers. When the
    /// first subscriber requests a resource, a watcher is created. Subsequent
    /// subscribers increment the counter, and the watcher is removed when the
    /// last subscriber unsubscribes.
    async fn create_watcher(&self, uri: String, path: PathBuf) -> Result<(), ResourceError> {
        let mut watchers = self.watchers.lock().await;

        // If watcher already exists, just increment the counter
        if let Some(handle) = watchers.get_mut(&uri) {
            handle.subscriber_count += 1;
            return Ok(());
        }

        // Create a channel to receive events
        let (tx, rx) = mpsc::channel(32);

        // Set up the file system watcher
        let watcher = self
            .setup_fs_watcher(tx, path.clone())
            .map_err(|e| ResourceError::Custom(format!("Failed to create file watcher: {}", e)))?;

        // Store the watcher
        watchers.insert(uri.clone(), WatcherHandle { _watcher: watcher, subscriber_count: 1 });

        // Spawn a task to process events for this resource
        let resource_manager = self.resource_manager.clone();
        let uri_clone = uri.clone();
        tokio::spawn(async move {
            Self::process_events(rx, resource_manager, uri_clone).await;
        });

        Ok(())
    }

    /// Configures and initializes a filesystem watcher for detecting changes
    ///
    /// Sets up the appropriate watch mode depending on whether the path is
    /// a file or directory. For files, we also watch the parent directory
    /// to detect if the file is deleted and recreated.
    fn setup_fs_watcher(&self, tx: Sender<Event>, path: PathBuf) -> Result<RecommendedWatcher, notify::Error> {
        let config = Config::default()
            .with_poll_interval(std::time::Duration::from_secs(1)) // Fallback for unsupported filesystems
            .with_compare_contents(true);

        let mut watcher = RecommendedWatcher::new(
            move |res: Result<Event, notify::Error>| {
                if let Ok(event) = res {
                    // Only forward relevant events
                    match event.kind {
                        EventKind::Create(_) | EventKind::Modify(_) | EventKind::Remove(_) => {
                            let _ = tx.blocking_send(event);
                        }
                        _ => {}
                    }
                }
            },
            config,
        )?;

        // Start watching the path
        if path.is_dir() {
            watcher.watch(&path, RecursiveMode::Recursive)?;
        } else {
            // For files, watch the file itself and its parent directory
            watcher.watch(&path, RecursiveMode::NonRecursive)?;
            if let Some(parent) = path.parent() {
                watcher.watch(parent, RecursiveMode::NonRecursive)?;
            }
        }

        Ok(watcher)
    }

    /// Processes filesystem events and notifies subscribers of changes
    ///
    /// Uses debouncing to prevent rapid-fire notifications when many changes
    /// occur at once, except for removal events which are sent immediately.
    async fn process_events(mut rx: Receiver<Event>, resource_manager: Arc<ResourceManager>, uri: String) {
        use tokio::time::{interval, Duration};

        // Use debouncing to avoid rapid-fire notifications
        let mut debounce_timer = interval(Duration::from_millis(300));
        let mut has_changes = false;

        loop {
            tokio::select! {
                Some(event) = rx.recv() => {
                    debug!("File event detected: {:?} for {}", event.kind, uri);
                    has_changes = true;

                    // Skip waiting if it's a deletion event
                    if matches!(event.kind, EventKind::Remove(_)) {
                        // Notify immediately for removals
                        if let Err(e) = resource_manager.notify_resource_updated(&uri).await {
                            error!("Failed to notify resource removed: {}", e);
                        }
                        has_changes = false;
                        continue;
                    }

                    // For other events, reset the debounce timer
                    debounce_timer = interval(Duration::from_millis(300));
                }

                _ = debounce_timer.tick() => {
                    if has_changes {
                        if let Err(e) = resource_manager.notify_resource_updated(&uri).await {
                            error!("Failed to notify resource updated: {}", e);
                        }
                        has_changes = false;
                    }
                }

                else => break,
            }
        }
    }

    /// Decrements the subscriber count for a watcher and removes it if no subscribers remain
    async fn remove_watcher(&self, uri: &str) -> Result<(), ResourceError> {
        let mut watchers = self.watchers.lock().await;

        if let Some(handle) = watchers.get_mut(uri) {
            handle.subscriber_count -= 1;

            // Remove the watcher if no subscribers left
            if handle.subscriber_count == 0 {
                watchers.remove(uri);
                debug!("Removed watcher for {}, no more subscribers", uri);
            }
        }

        Ok(())
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

        // Update the cache with current file metadata
        if metadata.is_file() {
            let last_modified = metadata
                .modified()
                .map_err(|e| ResourceError::Reading(format!("Failed to get last modified time: {}", e)))?;

            let mime_type = Self::guess_mime_type(&path);
            let is_binary = mime_type.as_ref().map_or(false, |mt| Self::is_binary(mt));

            let mut cache = self.cache.lock().unwrap();
            cache.insert(
                uri.clone(),
                FileMetadata { path: path.clone(), mime_type: mime_type.clone(), is_binary, last_modified },
            );
        }

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

    async fn subscribe(&self, uri: String, client_id: ClientId) -> Result<(), ResourceError> {
        // First, check if the resource exists
        let path = self.uri_to_path(&uri)?;

        if !path.exists() {
            return Err(ResourceError::NotFound(format!("Cannot subscribe to non-existent resource: {}", uri)));
        }

        // Add the subscriber
        self.resource_manager.add_subscriber(&uri, client_id.clone())?;

        // Set up watcher for this resource
        self.create_watcher(uri.clone(), path).await?;

        info!("Subscribed to resource: {} for client: {}", uri, client_id);
        Ok(())
    }

    async fn unsubscribe(&self, uri: String, client_id: ClientId) -> Result<(), ResourceError> {
        // Remove the subscriber
        self.resource_manager.remove_subscriber(&uri, client_id.clone())?;

        // Clean up the watcher if needed
        self.remove_watcher(&uri).await?;

        info!("Unsubscribed from resource: {} for client: {}", uri, client_id);
        Ok(())
    }

    fn provide_resource_manager(&self) -> Option<Arc<ResourceManager>> {
        Some(self.get_resource_manager())
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
    use crate::resources::NotificationCallback;

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
    async fn test_subscription_and_notification() {
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

        // Set up a notification callback
        let notification_received = Arc::new(StdMutex::new(false));
        let notification_received_clone = notification_received.clone();

        let callback: NotificationCallback = Box::new(move |client_id, params| {
            let notification_received = notification_received_clone.clone();
            Box::pin(async move {
                println!("Notification received for client {}: {:?}", client_id, params);
                let mut flag = notification_received.lock().unwrap();
                *flag = true;
                Ok(())
            })
        });

        fs_resource.resource_manager.set_notification_callback(callback).unwrap();

        let client_id = ClientId::new();

        // Subscribe to the file
        let uri = format!("file://{}", file_path.display());
        ResourceDef::subscribe(&fs_resource, uri.clone(), client_id.clone()).await.unwrap();

        // Check that we're subscribed
        assert!(fs_resource.resource_manager.has_subscribers(&uri).unwrap());

        // Simulate a file change notification
        // In a real scenario, this would be triggered by the file system watcher
        fs_resource.resource_manager.notify_resource_updated(&uri).await.unwrap();

        // Check that the notification was received
        assert!(*notification_received.lock().unwrap());

        // Unsubscribe
        ResourceDef::unsubscribe(&fs_resource, uri.clone(), client_id.clone()).await.unwrap();

        // Check that we're no longer subscribed
        assert!(!fs_resource.resource_manager.has_subscribers(&uri).unwrap());
    }
}
