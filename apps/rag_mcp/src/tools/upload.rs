use bioma_actor::Engine;
use bioma_mcp::{
    schema::CallToolResult,
    server::RequestContext,
    tools::{ToolDef, ToolError},
};
use reqwest::Client;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::Arc;
use tokio::fs;
use tracing::{error, info};
use url::Url;
use uuid::Uuid;
use zip::ZipArchive;

#[derive(JsonSchema, Debug, Serialize, Deserialize)]
pub struct UploadArgs {
    pub url: String,
    pub path: String,
}

#[derive(Serialize)]
pub struct UploadTool {
    #[serde(skip_serializing)]
    engine: Arc<Engine>,
}

#[derive(Serialize, Deserialize)]
struct Uploaded {
    message: String,
    paths: Vec<PathBuf>,
    size: usize,
}

impl UploadTool {
    pub fn new(engine: Arc<Engine>) -> Self {
        Self { engine }
    }
}

impl ToolDef for UploadTool {
    const NAME: &'static str = "upload";
    const DESCRIPTION: &'static str = "Upload files for indexing and retrieval by downloading from a URL";
    type Args = UploadArgs;

    async fn call(&self, args: Self::Args, _request_context: RequestContext) -> Result<CallToolResult, ToolError> {
        let client = Client::new();

        let url = Url::parse(&args.url).map_err(|e| ToolError::Execution(format!("Invalid URL: {}", e)))?;

        let output_dir = self.engine.local_store_dir().clone();

        let path = PathBuf::from(&args.path);
        let target_folder =
            if path.extension().is_some_and(|ext| ext == "zip") { path.with_extension("") } else { path };

        let dest_path = output_dir.join(&target_folder);

        if let Some(parent) = dest_path.parent() {
            fs::create_dir_all(parent)
                .await
                .map_err(|e| ToolError::Execution(format!("Failed to create target directory: {}", e)))?;
        }

        let temp_path = std::env::temp_dir().join(format!("upload_{}", Uuid::new_v4()));

        info!("Downloading file from {}", url);
        let response = client
            .get(url.clone())
            .send()
            .await
            .map_err(|e| ToolError::Execution(format!("Failed to download file: {}", e)))?;

        if !response.status().is_success() {
            return Err(ToolError::Execution(format!("Failed to download file, status code: {}", response.status())));
        }

        let bytes =
            response.bytes().await.map_err(|e| ToolError::Execution(format!("Failed to read response body: {}", e)))?;

        fs::write(&temp_path, bytes.clone())
            .await
            .map_err(|e| ToolError::Execution(format!("Failed to save downloaded file: {}", e)))?;

        let is_zip = url.path().ends_with(".zip") || (bytes.len() > 4 && &bytes[0..4] == b"PK\x03\x04");

        let result = if is_zip {
            let temp_path_clone = temp_path.clone();
            let target_folder_clone = target_folder.clone();
            let output_dir_clone = output_dir.clone();

            let extract_result = tokio::task::spawn_blocking(
                move || -> Result<Vec<PathBuf>, Box<dyn std::error::Error + Send + Sync>> {
                    let file = std::fs::File::open(&temp_path_clone)?;
                    let mut archive = ZipArchive::new(file)?;

                    let extraction_dir = output_dir_clone.join(&target_folder_clone);

                    let mut common_prefix: Option<PathBuf> = None;
                    for i in 0..archive.len() {
                        let file = archive.by_index(i)?;
                        let path = file.mangled_name();
                        let mut comps = path.components();
                        if let Some(first) = comps.next() {
                            let first_component = PathBuf::from(first.as_os_str());
                            match &common_prefix {
                                Some(cp) => {
                                    if *cp != first_component {
                                        common_prefix = None;
                                        break;
                                    }
                                }
                                None => {
                                    common_prefix = Some(first_component);
                                }
                            }
                        }
                    }

                    let mut extracted_files = Vec::new();

                    for i in 0..archive.len() {
                        let mut file = archive.by_index(i)?;
                        let mut outpath = file.mangled_name();

                        if let Some(ref cp) = common_prefix {
                            if let Ok(stripped) = outpath.strip_prefix(cp) {
                                outpath = stripped.to_path_buf();
                            }
                        }

                        let final_path = extraction_dir.join(&outpath);

                        if file.name().ends_with('/') {
                            std::fs::create_dir_all(&final_path)?;
                        } else {
                            if let Some(parent) = final_path.parent() {
                                std::fs::create_dir_all(parent)?;
                            }

                            let mut outfile = std::fs::File::create(&final_path)?;
                            std::io::copy(&mut file, &mut outfile)?;

                            if let Ok(relative) = final_path.strip_prefix(&output_dir_clone) {
                                extracted_files.push(relative.to_path_buf());
                            }
                        }
                    }

                    Ok(extracted_files)
                },
            )
            .await;

            let _ = fs::remove_file(&temp_path).await;

            match extract_result {
                Ok(Ok(files)) => Uploaded {
                    message: format!("Zip file extracted {} files successfully", files.len()),
                    paths: files,
                    size: bytes.len(),
                },
                Ok(Err(e)) => {
                    error!("Error extracting zip file: {:?}", e);
                    return Err(ToolError::Execution(format!("Failed to extract zip archive: {}", e)));
                }
                Err(e) => {
                    error!("Task error: {:?}", e);
                    return Err(ToolError::Execution(format!("Internal error during extraction: {}", e)));
                }
            }
        } else {
            fs::copy(&temp_path, &dest_path)
                .await
                .map_err(|e| ToolError::Execution(format!("Failed to save file: {}", e)))?;

            let _ = fs::remove_file(&temp_path).await;

            let relative_path = dest_path.strip_prefix(&output_dir).unwrap_or(&dest_path).to_path_buf();
            Uploaded {
                message: "File downloaded and saved successfully".to_string(),
                paths: vec![relative_path],
                size: bytes.len(),
            }
        };

        let response_value = serde_json::to_value(result).unwrap();
        Ok(CallToolResult { meta: None, content: vec![response_value], is_error: None })
    }
}
