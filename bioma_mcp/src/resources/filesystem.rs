use crate::resources::{ResourceDef, ResourceError};
use crate::schema::{ReadResourceResult, ResourceTemplate, ResourceUpdatedNotificationParams};
use crate::server::Context;
use notify::{Config, Event, RecommendedWatcher, RecursiveMode, Watcher};
use serde::Serialize;
use std::any::Any;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
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

        let path = Path::new(path_str.trim_start_matches('/'));
        let absolute_path = self.base_dir.join(path);

        if !absolute_path.exists() {
            return Err(ResourceError::NotFound(format!("File not found: {}", absolute_path.display())));
        }

        let canonical_base = self
            .base_dir
            .canonicalize()
            .map_err(|e| ResourceError::Custom(format!("Failed to canonicalize base directory: {}", e)))?;

        let canonical_path = absolute_path.canonicalize().map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                return ResourceError::NotFound(format!("File not found: {}", absolute_path.display()));
            }
            ResourceError::Custom(format!("Failed to canonicalize path: {}", e))
        })?;

        if !canonical_path.starts_with(&canonical_base) && canonical_path != canonical_base {
            return Err(ResourceError::Custom(format!(
                "Access denied: {} is outside the base directory",
                absolute_path.display()
            )));
        }

        Ok(canonical_path)
    }

    fn path_to_uri(&self, path: &Path) -> String {
        let path_str = path.to_string_lossy();
        format!("file://{}", path_str)
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

        // Create initial file
        {
            let mut file = File::create(&file_path).unwrap();
            write!(file, "Initial content").unwrap();
        }

        let relative_path = file_path.strip_prefix(temp_dir.path()).unwrap();
        let uri = format!("file:///{}", relative_path.display());

        println!("uri: {}", uri);

        // Subscribe to the file
        fs_resource.subscribe(uri.clone()).await.unwrap();

        // Modify the file
        {
            let mut file = File::create(&file_path).unwrap();
            write!(file, "Updated content").unwrap();
        }

        // Wait a bit for the notification to be processed
        tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

        // Unsubscribe from the file
        fs_resource.unsubscribe(uri.clone()).await.unwrap();

        // Verify the file content was updated
        let result = fs_resource.read(uri.clone()).await.unwrap();
        let content = &result.contents[0];
        let text = content["text"].as_str().expect("text field should be a string");
        assert_eq!(text, "Updated content");

        // Verify the watcher was cleaned up
        let watchers = fs_resource.watchers.lock().await;
        assert!(!watchers.contains_key(&uri));
    }
}
