use crate::resources::{ResourceCompletionHandler, ResourceDef, ResourceError};
use crate::schema::{ReadResourceResult, ResourceTemplate, ResourceUpdatedNotificationParams};
use crate::server::Context;
use notify::{Config, Event, RecommendedWatcher, RecursiveMode, Watcher};
use serde::Serialize;
use std::any::Any;
use std::collections::HashMap;
use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::Arc;
use tokio::fs;
use tokio::io::AsyncReadExt;
use tokio::sync::{mpsc, Mutex};
use tokio::task::JoinHandle;
use tracing::info;
use url::Url;

#[derive(Clone, Serialize)]
pub struct FileSystem {
    #[serde(skip)]
    context: Context,

    #[serde(skip)]
    base_dir: Arc<PathBuf>,

    #[serde(skip)]
    watchers: Arc<Mutex<HashMap<String, JoinHandle<()>>>>,
}

impl FileSystem {
    pub fn new<P: AsRef<Path>>(base_dir: P, context: Context) -> Self {
        Self {
            base_dir: Arc::new(base_dir.as_ref().to_path_buf()),
            watchers: Arc::new(Mutex::new(HashMap::new())),
            context,
        }
    }

    async fn watch_path(path: impl AsRef<Path>, tx: mpsc::Sender<()>) -> Result<JoinHandle<()>, ResourceError> {
        let path = path.as_ref().to_owned();

        let handle = tokio::spawn(async move {
            let (watcher_tx, mut watcher_rx) = mpsc::channel(16);

            let mut watcher = RecommendedWatcher::new(
                move |res: notify::Result<Event>| {
                    if res.is_ok() {
                        let _ = watcher_tx.blocking_send(());
                    }
                },
                Config::default(),
            )
            .unwrap();

            // TODO: End this task when tx is dropped

            watcher.watch(&path, RecursiveMode::Recursive).unwrap();

            while watcher_rx.recv().await.is_some() {
                if tx.send(()).await.is_err() {
                    break;
                }
            }
        });

        Ok(handle)
    }

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

    fn is_binary(mime_type: &str) -> bool {
        !mime_type.starts_with("text/")
            && !mime_type.contains("json")
            && !mime_type.contains("xml")
            && !mime_type.contains("javascript")
    }

    fn uri_to_path(&self, uri: &str) -> Result<PathBuf, ResourceError> {
        let url = Url::parse(uri).map_err(|e| ResourceError::Custom(format!("Invalid URI: {}", e)))?;

        if url.scheme() != "file" {
            return Err(ResourceError::Custom(format!("Unsupported URI scheme: {}", url.scheme())));
        }

        let path_str = url.path();

        if path_str.is_empty() || path_str == "/" {
            return Ok((*self.base_dir).clone());
        }

        let absolute_path = PathBuf::from(path_str);

        if absolute_path.exists() {
            let canonical_base = self
                .base_dir
                .canonicalize()
                .map_err(|e| ResourceError::Custom(format!("Failed to canonicalize base directory: {}", e)))?;

            let canonical_path = absolute_path
                .canonicalize()
                .map_err(|e| ResourceError::Custom(format!("Failed to canonicalize path: {}", e)))?;

            if canonical_path.starts_with(&canonical_base) || canonical_path == canonical_base {
                return Ok(canonical_path);
            } else {
                return Err(ResourceError::Custom(format!(
                    "Access denied: {} is outside the base directory",
                    absolute_path.display()
                )));
            }
        }

        let relative_path = path_str.trim_start_matches('/');
        let joined_path = self.base_dir.join(relative_path);

        if !joined_path.exists() {
            return Err(ResourceError::NotFound(format!("File not found: {}", joined_path.display())));
        }

        let canonical_base = self
            .base_dir
            .canonicalize()
            .map_err(|e| ResourceError::Custom(format!("Failed to canonicalize base directory: {}", e)))?;

        let canonical_path = joined_path.canonicalize().map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                return ResourceError::NotFound(format!("File not found: {}", joined_path.display()));
            }
            ResourceError::Custom(format!("Failed to canonicalize path: {}", e))
        })?;

        if !canonical_path.starts_with(&canonical_base) && canonical_path != canonical_base {
            return Err(ResourceError::Custom(format!(
                "Access denied: {} is outside the base directory",
                joined_path.display()
            )));
        }

        Ok(canonical_path)
    }

    fn path_to_uri(&self, path: &Path) -> String {
        let path_str = path.to_string_lossy();
        format!("file://{}", path_str)
    }
}

impl ResourceCompletionHandler for FileSystem {
    fn complete_argument<'a>(
        &'a self,
        argument_name: &'a str,
        argument_value: &'a str,
    ) -> Pin<Box<dyn Future<Output = Result<Vec<String>, ResourceError>> + Send + 'a>> {
        Box::pin(async move {
            // Only handle path argument completions
            if argument_name != "path" {
                return Ok(vec![]);
            }

            // Normalize the input path
            let input_path = if argument_value.starts_with('/') {
                argument_value.to_string()
            } else {
                format!("/{}", argument_value)
            };

            // Determine the directory we should look in and the prefix to match
            let (dir_to_search, prefix_to_match) = if input_path.ends_with('/') || input_path == "/" {
                // If the path ends with a slash, we're looking for contents of that directory
                (input_path.clone(), "".to_string())
            } else {
                // Otherwise, we're looking for completions of the last part
                let parent = if let Some(idx) = input_path.rfind('/') {
                    input_path[..=idx].to_string()
                } else {
                    "/".to_string()
                };

                let file_prefix = if let Some(idx) = input_path.rfind('/') {
                    input_path[(idx + 1)..].to_string()
                } else {
                    input_path.clone()
                };

                (parent, file_prefix)
            };

            // Convert to actual filesystem path
            let fs_path_to_search = if dir_to_search == "/" {
                (*self.base_dir).clone()
            } else {
                let relative_path = dir_to_search.trim_start_matches('/');
                self.base_dir.join(relative_path)
            };

            // Check if directory exists
            if !fs_path_to_search.exists() || !fs_path_to_search.is_dir() {
                return Ok(vec![]);
            }

            // Read directory entries
            let mut entries = match fs::read_dir(&fs_path_to_search).await {
                Ok(entries) => entries,
                Err(_) => return Ok(vec![]),
            };

            let mut completions = Vec::new();

            // Process each entry
            while let Some(entry) = entries.next_entry().await.ok().flatten() {
                let file_name = match entry.file_name().to_str() {
                    Some(name) => name.to_string(),
                    None => continue,
                };

                // Skip hidden files and directories unless explicitly searching for them
                if file_name.starts_with('.') && !prefix_to_match.starts_with('.') {
                    continue;
                }

                // Check if entry matches the prefix
                if prefix_to_match.is_empty() || file_name.to_lowercase().starts_with(&prefix_to_match.to_lowercase()) {
                    let is_dir = match entry.file_type().await {
                        Ok(file_type) => file_type.is_dir(),
                        Err(_) => false,
                    };

                    // Format the suggestion
                    let mut suggestion = if dir_to_search == "/" {
                        file_name
                    } else {
                        let parent_path = dir_to_search.trim_end_matches('/');
                        format!("{}/{}", parent_path.trim_start_matches('/'), file_name)
                    };

                    // Add trailing slash for directories
                    if is_dir {
                        suggestion.push('/');
                    }

                    completions.push(suggestion);
                }
            }

            Ok(completions)
        })
    }
}

impl ResourceDef for FileSystem {
    const NAME: &'static str = "filesystem";
    const DESCRIPTION: &'static str = "Access files from the server's filesystem";
    const URI: &'static str = "file:///";
    const MIME_TYPE: Option<&'static str> = None;

    fn templates() -> Vec<ResourceTemplate> {
        vec![ResourceTemplate {
            name: Self::NAME.to_string(),
            description: Some(Self::DESCRIPTION.to_string()),
            uri_template: "file:///{path}".to_string(),
            mime_type: None,
            annotations: None,
        }]
    }

    async fn read(&self, uri: String) -> Result<ReadResourceResult, ResourceError> {
        let path = self.uri_to_path(&uri)?;

        let metadata = fs::metadata(&path).await.map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                ResourceError::NotFound(format!("File not found: {}", path.display()))
            } else {
                ResourceError::Reading(format!("Failed to read file metadata: {}", e))
            }
        })?;

        if metadata.is_dir() {
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
            let mime_type = Self::guess_mime_type(&path);
            let is_binary = mime_type.as_ref().map_or(false, |mt| Self::is_binary(mt));

            if is_binary {
                let mut file = fs::File::open(&path)
                    .await
                    .map_err(|e| ResourceError::Reading(format!("Failed to open file: {}", e)))?;

                let mut buffer = Vec::new();
                file.read_to_end(&mut buffer)
                    .await
                    .map_err(|e| ResourceError::Reading(format!("Failed to read file: {}", e)))?;

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

    async fn subscribe(&self, uri: String) -> Result<(), ResourceError> {
        let path = self.uri_to_path(&uri)?;
        if !path.exists() {
            return Err(ResourceError::NotFound(format!("Cannot subscribe to non-existent resource: {}", uri)));
        }

        let (on_resource_updated_tx, mut on_resource_updated_rx) = mpsc::channel(1);
        let context = self.context.clone();
        let updated_uri = uri.clone();
        tokio::spawn(async move {
            while let Some(_) = on_resource_updated_rx.recv().await {
                let _ = context.resource_updated(ResourceUpdatedNotificationParams { uri: updated_uri.clone() }).await;
            }
        });

        let handle = FileSystem::watch_path(path, on_resource_updated_tx).await?;
        self.watchers.lock().await.insert(uri.clone(), handle);
        info!("Subscribed to resource: {}", uri);
        Ok(())
    }

    async fn unsubscribe(&self, uri: String) -> Result<(), ResourceError> {
        let mut watchers = self.watchers.lock().await;
        if let Some(handle) = watchers.remove(&uri) {
            handle.abort();
            info!("Unsubscribed from resource: {}", uri);
            Ok(())
        } else {
            info!("Already unsubscribed from resource: {}", uri);
            Ok(())
        }
    }
}

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
        let temp_dir = tempdir().unwrap();
        let file_path = temp_dir.path().join("test.txt");

        {
            let mut file = File::create(&file_path).unwrap();
            write!(file, "Hello, world!").unwrap();
        }

        let fs_resource = FileSystem::new(temp_dir.path(), Context::test());

        let relative_path = file_path.strip_prefix(temp_dir.path()).unwrap();
        let uri = format!("file:///{}", relative_path.display());

        let result = fs_resource.read(uri).await.unwrap();

        let content = &result.contents[0];
        let text = content["text"].as_str().expect("text field should be a string");
        assert_eq!(text, "Hello, world!");
    }

    #[tokio::test]
    async fn test_filesystem_not_found() {
        let temp_dir = tempdir().unwrap();

        let fs_resource = FileSystem::new(temp_dir.path(), Context::test());

        let uri = format!("file://{}/nonexistent.txt", temp_dir.path().display());
        let result = fs_resource.read(uri).await;

        assert!(result.is_err());
        match result {
            Err(ResourceError::NotFound(_)) => {}
            _ => panic!("Expected NotFound error"),
        }
    }

    #[tokio::test]
    async fn test_filesystem_read_directory() {
        let temp_dir = tempdir().unwrap();

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

        let fs_resource = FileSystem::new(temp_dir.path(), Context::test());

        let uri = "file:///";
        let result = fs_resource.read(uri.to_string()).await.unwrap();

        assert_eq!(result.contents.len(), 3);

        let entries: Vec<&str> = result.contents.iter().map(|entry| entry["text"].as_str().unwrap()).collect();

        assert!(entries.contains(&"test1.txt"));
        assert!(entries.contains(&"test2.txt"));
        assert!(entries.contains(&"subdir/"));
    }

    #[tokio::test]
    async fn test_filesystem_root_uri() {
        let temp_dir = tempdir().unwrap();

        let file_path = temp_dir.path().join("test.txt");
        {
            let mut file = File::create(&file_path).unwrap();
            write!(file, "Root test").unwrap();
        }

        let fs_resource = FileSystem::new(temp_dir.path(), Context::test());

        let result = fs_resource.read("file:///".to_string()).await.unwrap();

        let entries: Vec<&str> = result.contents.iter().map(|entry| entry["text"].as_str().unwrap()).collect();
        assert!(entries.contains(&"test.txt"));
    }

    #[tokio::test]
    async fn test_uri_to_path_root() {
        let temp_dir = tempdir().unwrap();

        let fs_resource = FileSystem::new(temp_dir.path(), Context::test());

        let path = fs_resource.uri_to_path("file:///").unwrap();
        assert_eq!(path, temp_dir.path());

        let path2 = fs_resource.uri_to_path("file://").unwrap();
        assert_eq!(path2, temp_dir.path());
    }

    #[tokio::test]
    async fn test_path_to_uri() {
        let temp_dir = tempdir().unwrap();

        let fs_resource = FileSystem::new(temp_dir.path(), Context::test());

        let uri = fs_resource.path_to_uri(temp_dir.path());
        assert!(uri.starts_with("file://"));
        assert!(uri.ends_with(&*temp_dir.path().to_string_lossy()));
    }

    #[tokio::test]
    async fn test_nonexistent_directory() {
        let temp_dir = tempdir().unwrap();

        let fs_resource = FileSystem::new(temp_dir.path(), Context::test());

        let nonexistent_dir = format!("file://{}/nonexistent_dir/", temp_dir.path().display());
        let result = fs_resource.read(nonexistent_dir).await;

        assert!(result.is_err());
        match result {
            Err(ResourceError::NotFound(_)) => {}
            _ => panic!("Expected NotFound error"),
        }
    }

    #[tokio::test]
    async fn test_subscription_and_notification() {
        let temp_dir = tempdir().unwrap();
        let file_path = temp_dir.path().join("test.txt");
        let fs_resource = FileSystem::new(temp_dir.path(), Context::test());

        {
            let mut file = File::create(&file_path).unwrap();
            write!(file, "Initial content").unwrap();
        }

        let relative_path = file_path.strip_prefix(temp_dir.path()).unwrap();
        let uri = format!("file:///{}", relative_path.display());

        fs_resource.subscribe(uri.clone()).await.unwrap();

        {
            let mut file = File::create(&file_path).unwrap();
            write!(file, "Updated content").unwrap();
        }

        tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

        fs_resource.unsubscribe(uri.clone()).await.unwrap();

        let result = fs_resource.read(uri.clone()).await.unwrap();
        let content = &result.contents[0];
        let text = content["text"].as_str().expect("text field should be a string");
        assert_eq!(text, "Updated content");

        let watchers = fs_resource.watchers.lock().await;
        assert!(!watchers.contains_key(&uri));
    }

    #[tokio::test]
    async fn test_filesystem_absolute_path() {
        let temp_dir = tempdir().unwrap();
        let file_path = temp_dir.path().join("test.txt");

        {
            let mut file = File::create(&file_path).unwrap();
            write!(file, "Hello, absolute path!").unwrap();
        }

        let fs_resource = FileSystem::new(temp_dir.path(), Context::test());

        let uri = format!("file://{}", file_path.display());
        let result = fs_resource.read(uri).await.unwrap();

        let content = &result.contents[0];
        let text = content["text"].as_str().expect("text field should be a string");
        assert_eq!(text, "Hello, absolute path!");

        let outside_temp_dir = tempdir().unwrap();
        let outside_file_path = outside_temp_dir.path().join("outside.txt");

        {
            let mut file = File::create(&outside_file_path).unwrap();
            write!(file, "I'm outside the base directory!").unwrap();
        }

        let outside_uri = format!("file://{}", outside_file_path.display());
        let result = fs_resource.read(outside_uri).await;

        assert!(result.is_err());
        match result {
            Err(ResourceError::Custom(msg)) if msg.contains("Access denied") => {}
            _ => panic!("Expected Access denied error"),
        }
    }

    #[tokio::test]
    async fn test_path_completion_empty() {
        let temp_dir = tempdir().unwrap();

        // Create test directory structure
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

        let fs_resource = FileSystem::new(temp_dir.path(), Context::test());

        // Test completion with empty input (should list root)
        let completions = fs_resource.complete_argument("path", "").await.unwrap();

        assert!(completions.contains(&"test1.txt".to_string()));
        assert!(completions.contains(&"test2.txt".to_string()));
        assert!(completions.contains(&"subdir/".to_string()));
    }
}
