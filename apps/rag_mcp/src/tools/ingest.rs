use anyhow::{Error, anyhow};
use bioma_actor::Engine;
use bioma_mcp::{schema::CallToolResult, server::RequestContext, tools::ToolDef};
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
pub struct IngestArgs {
    pub url: String,
    pub path: String,
}

#[derive(Serialize)]
pub struct IngestTool {
    #[serde(skip_serializing)]
    engine: Arc<Engine>,
}

#[derive(Serialize, Deserialize)]
struct Uploaded {
    message: String,
    paths: Vec<PathBuf>,
    size: usize,
}

impl IngestTool {
    pub fn new(engine: Arc<Engine>) -> Self {
        Self { engine }
    }
}

impl ToolDef for IngestTool {
    const NAME: &'static str = "upload";
    const DESCRIPTION: &'static str = "Upload files for indexing and retrieval by downloading from a URL";
    type Args = IngestArgs;

    async fn call(&self, args: Self::Args, _request_context: RequestContext) -> Result<CallToolResult, Error> {
        let client = Client::new();

        let url = Url::parse(&args.url)?;

        let output_dir = self.engine.local_store_dir().clone();

        let path = PathBuf::from(&args.path);
        let target_folder =
            if path.extension().is_some_and(|ext| ext == "zip") { path.with_extension("") } else { path };

        let dest_path = output_dir.join(&target_folder);

        if let Some(parent) = dest_path.parent() {
            fs::create_dir_all(parent).await?;
        }

        let temp_path = std::env::temp_dir().join(format!("upload_{}", Uuid::new_v4()));

        info!("Downloading file from {}", url);
        let response = client.get(url.clone()).send().await?;

        if !response.status().is_success() {
            return Err(anyhow!("Failed to download file, status code: {}", response.status()));
        }

        let bytes = response.bytes().await?;

        fs::write(&temp_path, bytes.clone()).await?;

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
                    return Err(anyhow!("Failed to extract zip archive: {}", e));
                }
                Err(e) => {
                    error!("Task error: {:?}", e);
                    return Err(anyhow!("Internal error during extraction: {}", e));
                }
            }
        } else {
            fs::copy(&temp_path, &dest_path).await?;

            let _ = fs::remove_file(&temp_path).await;

            let relative_path = dest_path.strip_prefix(&output_dir).unwrap_or(&dest_path).to_path_buf();
            Uploaded {
                message: "File downloaded and saved successfully".to_string(),
                paths: vec![relative_path],
                size: bytes.len(),
            }
        };

        let response_value = serde_json::to_value(result)?;
        Ok(CallToolResult { meta: None, content: vec![response_value], is_error: None })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use std::sync::Arc;
    use wiremock::{
        Mock, MockServer, ResponseTemplate,
        matchers::{method, path},
    };
    use zip::write::FileOptions;

    #[tokio::test]
    async fn download_plain_file() {
        let server = MockServer::start().await;
        let body = b"Hello world!";
        Mock::given(method("GET"))
            .and(path("/file.txt"))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(body))
            .mount(&server)
            .await;

        let engine = Engine::test().await.unwrap();
        let tool = IngestTool::new(Arc::new(engine.clone()));

        let args = IngestArgs { url: format!("{}/file.txt", server.uri()), path: "greetings/file.txt".into() };
        let result = tool.call(args, RequestContext::default()).await.unwrap();

        let uploaded: Uploaded = serde_json::from_value(result.content[0].clone()).unwrap();
        assert_eq!(uploaded.paths[0], PathBuf::from("greetings/file.txt"));

        let abs_path = engine.local_store_dir().join(&uploaded.paths[0]);
        let bytes_on_disk = tokio::fs::read(&abs_path).await.unwrap();
        assert_eq!(bytes_on_disk.as_slice(), body);
        assert_eq!(uploaded.size, body.len());

        let _ = tokio::fs::remove_file(abs_path).await;
    }

    #[tokio::test]
    async fn download_and_extract_zip() {
        let zip_bytes = {
            let mut buf = std::io::Cursor::new(Vec::new());
            {
                let mut zw = zip::ZipWriter::new(&mut buf);
                let opts: FileOptions<'_, ()> = FileOptions::default();
                for &(name, bytes) in &[("folder/a.txt", b"A" as &[u8]), ("folder/b/b.txt", b"B" as &[u8])] {
                    zw.start_file(name, opts).unwrap();
                    zw.write_all(bytes).unwrap();
                }
                zw.finish().unwrap();
            }
            buf.into_inner()
        };

        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/archive.zip"))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(zip_bytes))
            .mount(&server)
            .await;

        let engine = Engine::test().await.unwrap();
        let tool = IngestTool::new(Arc::new(engine.clone()));

        let args = IngestArgs { url: format!("{}/archive.zip", server.uri()), path: "archive.zip".into() };
        let result = tool.call(args, RequestContext::default()).await.unwrap();

        let uploaded: Uploaded = serde_json::from_value(result.content[0].clone()).unwrap();
        assert_eq!(uploaded.paths.len(), 2, "two files extracted");

        assert!(uploaded.paths.contains(&PathBuf::from("archive/a.txt")));
        assert!(uploaded.paths.contains(&PathBuf::from("archive/b/b.txt")));

        let abs_a = engine.local_store_dir().join("archive/a.txt");
        let data_a = tokio::fs::read(&abs_a).await.unwrap();
        assert_eq!(data_a.as_slice(), b"A");

        let _ = tokio::fs::remove_dir_all(engine.local_store_dir().join("archive")).await;
    }
}
