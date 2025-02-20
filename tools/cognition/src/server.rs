use actix_cors::Cors;
use actix_files::{Files, NamedFile};
use actix_multipart::form::MultipartForm;
use actix_web::{
    http::Method,
    middleware::Logger,
    web::{self, Json},
    App, HttpResponse, HttpServer, Responder,
};
use api_schema::{
    AskQueryRequest, ChatQueryRequest, EmbeddingsQueryRequest, IndexRequest, ModelEmbed, RetrieveContextRequest,
    RetrieveOutputFormat, ThinkQueryRequest, UploadRequest,
};
use base64::Engine as Base64Engine;
use bioma_actor::prelude::*;
use bioma_llm::{indexer::Indexed, markitdown::MarkitDown, pdf_analyzer::PdfAnalyzer, retriever::ListedSources};
use bioma_llm::{prelude::*, retriever::ListSources};
use bioma_tool::client::ListTools;
use clap::Parser;
use cognition::{
    health_check::{
        check_markitdown, check_minio, check_ollama, check_pdf_analyzer, check_surrealdb, Responses, Service,
    },
    ChatResponse, ToolHubMap, ToolsHub, UserActor,
};
use embeddings::EmbeddingContent;
use futures_util::StreamExt;
use indexer::Metadata;
use ollama_rs::generation::{options::GenerationOptions, tools::ToolInfo};
use serde::Serialize;
use serde_json::json;
use server_config::{Args, ServerConfig};
use std::{collections::HashMap, error::Error as StdError, time::Duration};
use tracing::{debug, error, info};
use url::Url;
use utoipa::{openapi::ServerBuilder, OpenApi};

mod api_schema;
mod server_config;

const UPLOAD_MEMORY_LIMIT: usize = 50 * 1024 * 1024; // 50MB in bytes
const UPLOAD_TOTAL_LIMIT: usize = 100 * 1024 * 1024; // 100MB in bytes

struct AppState {
    config: ServerConfig,
    engine: Engine,
    indexer: ActorId,
    retriever: ActorId,
    embeddings: ActorId,
    rerank: ActorId,
    chat: ActorId,
    think_chat: ActorId,
}

impl AppState {
    async fn user_actor(&self) -> Result<ActorContext<UserActor>, SystemActorError> {
        // For now, we use a random actor id to identify the user.
        // In the future, we will use cookies or other methods to identify the user.
        let ulid = ulid::Ulid::new();
        let prefix_id = "/rag/user/";
        let actor_id = ActorId::of::<UserActor>(format!("{}{}", prefix_id, ulid.to_string()));
        let user_actor = UserActor {};
        let (ctx, _) = Actor::spawn(
            self.engine.clone(),
            actor_id,
            user_actor,
            SpawnOptions::builder().exists(SpawnExistsOptions::Restore).build(),
        )
        .await?;
        Ok(ctx)
    }
}

#[derive(utoipa::ToSchema, Serialize, Clone, Debug)]
#[schema(example = json!({
    "services": {
        "surrealdb": {
            "status": {
                "is_healthy": true,
                "error": null
            }
        },
        "ollama": {
            "status": {
                "is_healthy": true,
                "error": null
            },
            "health": {
                "models": [
                    {
                        "size_vram": 12345,
                        "model": "llama2"
                    }
                ]
            }
        }
    }
}))]
pub struct HealthCheckResponse {
    services: HashMap<Service, Responses>,
}

#[utoipa::path(
    get,
    path = "/health",
    description = "Check health of the server and its services.",
    responses(
        (status = 200, description = "Health check response containing status of all services", body = HashMap<Service, Responses>)
    )
)]
async fn health(data: web::Data<AppState>) -> impl Responder {
    let mut services: HashMap<Service, Responses> = HashMap::new();

    // SurrealDB health check
    services.insert(Service::SurrealDB, check_surrealdb(data.config.engine.endpoint.to_string()).await);

    // Ollama health check
    services.insert(Service::Ollama, check_ollama(data.config.chat_endpoint.clone()).await);

    // pdf-analyzer health check
    services.insert(Service::PdfAnalyzer, check_pdf_analyzer(PdfAnalyzer::default().pdf_analyzer_url).await);

    // Markitdown health check
    let markitdown_check = check_markitdown(MarkitDown::default().markitdown_url).await;
    services.insert(Service::Markitdown, markitdown_check);

    // Minio health check
    let minio_url = Url::parse("http://127.0.0.1:9000").unwrap();
    services.insert(Service::Minio, check_minio(minio_url).await);

    HttpResponse::Ok().json(services)
}

#[utoipa::path(
    get,
    path = "/hello",
    description = "Hello, world! from the server.",
    responses(
        (status = 200, description = "Ok", body = String, content_type = "text/plain"),
    )
)]
async fn hello() -> impl Responder {
    HttpResponse::Ok().json("Hello world!")
}

#[utoipa::path(
    get,
    path = "/reset",
    description = "Reset bioma engine.",
    responses(
        (status = 200, description = "Ok", body = String, content_type = "text/plain"),
    )
)]
async fn reset(data: web::Data<AppState>) -> HttpResponse {
    info!("Resetting the engine");
    let engine = data.engine.clone();
    let res = engine.reset().await;
    match res {
        Ok(_) => {
            info!("Engine reset completed");
            HttpResponse::Ok().body("OK")
        }
        Err(e) => {
            error!("Error resetting engine: {:?}", e);
            HttpResponse::InternalServerError().body(e.to_string())
        }
    }
}

#[derive(utoipa::ToResponse, utoipa::ToSchema, Debug, Serialize)]
#[response(example = json!({
    "message": "File uploaded successfully",
    "paths": ["/path/to/file.txt"],
    "size": 1024
}))]
struct Uploaded {
    /// A message describing the result of the upload operation
    message: String,
    /// List of paths where files were saved, relative to the workspace root
    #[schema(value_type = Vec<String>)]
    paths: Vec<std::path::PathBuf>,
    /// Total size of uploaded files in bytes
    size: usize,
}

impl Default for UploadConfig {
    fn default() -> Self {
        Self { max_file_size: UPLOAD_TOTAL_LIMIT, max_memory_buffer: UPLOAD_MEMORY_LIMIT }
    }
}

/// Uploads a file to the server and processes it based on its type and provided metadata.
///
/// This endpoint accepts a multipart form-data request containing:
/// - **file**: The file to be uploaded.
/// - **metadata**: A JSON object with a `path` field specifying the target folder.
///
/// ### How It Works
///
/// 1. **Target Folder Determination:**
///    - If the metadata path ends with `.zip` (e.g. `"new.zip"`), the extension is stripped so that the
///      folder name becomes `"new"`.
///    - Otherwise, the provided path is used directly.
///
/// 2. **File Processing:**
///    - **Zip Files:**  
///      If the uploaded file's original filename ends with `.zip`, it is treated as a zip archive:
///        - The file is first copied to a temporary location.
///        - The zip archive is opened, and the function scans all entries to check for a common top-level
///          folder. During extraction, that common prefix (if present) is stripped.
///        - Files are extracted directly into `output_dir/target_folder` (using [`ZipFile::mangled_name`]) and only file paths (not directory entries) are collected for the response.
///    - **Non-Zip Files:**  
///      The file is simply copied into the destination folder.
///
/// ### Examples
///
/// - **Zip File with Metadata `"new.zip"`:**
///   - **Metadata:** `{"path": "new.zip"}`  
///   - **Uploaded File:** A zip file whose entries are:
///     - `generated-api/index.ts`
///     - `generated-api/api.ts`
///   - **Result:**  
///     - The target folder becomes `"new"`.
///     - Files are extracted into `output_dir/new/` so that:
///       - `generated-api/index.ts` → `output_dir/new/index.ts`
///       - `generated-api/api.ts`   → `output_dir/new/api.ts`
///     - Only the file paths (e.g. `new/index.ts` and `new/api.ts`) are returned in the response.
///
/// - **Non-Zip File with Metadata `"uploads"`:**
///   - **Metadata:** `{"path": "uploads"}`  
///   - **Uploaded File:** A non-zip file (e.g. `document.txt`).
///   - **Result:**  
///     - The file is copied directly into `output_dir/uploads/document.txt`, and that path is returned.
///     - Only the file path (e.g. `uploads/document.txt`) is returned in the response.
#[utoipa::path(
    post,
    path = "/upload",
    description = "Upload files to the server.",
    request_body(content = UploadRequest, content_type = "multipart/form-data"),
    responses(
        (status = 200, description = "File uploaded successfully", body = Uploaded, content_type = "application/json", examples(
            ("single_file" = (summary = "Single file upload", value = json!({
                "message": "File uploaded successfully",
                "paths": ["/path/to/file.txt"],
                "size": 1024
            }))),
            ("zip_file" = (summary = "Zip file extraction", value = json!({
                "message": "Zip file extracted 3 files successfully",
                "paths": ["/path/to/extracted/file1.txt", "/path/to/extracted/file2.txt", "/path/to/extracted/file3.txt"],
                "size": 5120
            })))
        )),
        (status = 500, description = "Internal server error")
    )
)]
async fn upload(MultipartForm(form): MultipartForm<UploadRequest>, data: web::Data<AppState>) -> impl Responder {
    let output_dir = data.engine.local_store_dir().clone();

    // Determine the target folder.
    // If the metadata path ends with ".zip", remove the extension so that "new.zip" becomes "new".
    let target_folder = if form.metadata.path.as_path().extension().map_or(false, |ext| ext == "zip") {
        form.metadata.path.with_extension("")
    } else {
        form.metadata.path.clone()
    };

    // Compute the destination path using the target folder.
    let dest_path = output_dir.join(&target_folder);

    // Create the parent directory for the destination if it doesn't exist.
    if let Some(parent) = dest_path.parent() {
        if let Err(e) = tokio::fs::create_dir_all(parent).await {
            error!("Failed to create target directory: {}", e);
            return HttpResponse::InternalServerError().json(json!({
                "error": "Failed to create target directory",
                "details": e.to_string()
            }));
        }
    }

    // Create a temporary file path for the uploaded file.
    let temp_path = std::env::temp_dir().join(format!("upload_{}", uuid::Uuid::new_v4()));

    // Copy the uploaded file to the temporary location.
    if let Err(e) = tokio::fs::copy(&form.file.file.path(), &temp_path).await {
        error!("Failed to copy uploaded file: {}", e);
        return HttpResponse::InternalServerError().json(json!({
            "error": "Failed to copy uploaded file",
            "details": e.to_string()
        }));
    }

    let temp_path_clone = temp_path.clone();
    let target_folder_clone = target_folder.clone();
    let output_dir_clone = output_dir.clone();

    if form.file.file_name.as_ref().map_or(false, |name| name.ends_with(".zip")) {
        // Handle zip file extraction.
        let extract_result = tokio::task::spawn_blocking(
            move || -> Result<Vec<std::path::PathBuf>, Box<dyn std::error::Error + Send + Sync>> {
                let file = std::fs::File::open(&temp_path_clone)?;
                let mut archive = zip::ZipArchive::new(file)?;

                // Define the extraction directory as output_dir/target_folder.
                let extraction_dir = output_dir_clone.join(&target_folder_clone);

                // Determine a common top-level prefix, if one exists.
                let mut common_prefix: Option<std::path::PathBuf> = None;
                for i in 0..archive.len() {
                    let file = archive.by_index(i)?;
                    // Use mangled_name() instead of sanitized_name() to avoid deprecation.
                    let path = file.mangled_name();
                    let mut comps = path.components();
                    if let Some(first) = comps.next() {
                        let first_component = std::path::PathBuf::from(first.as_os_str());
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

                // Now perform extraction.
                for i in 0..archive.len() {
                    let mut file = archive.by_index(i)?;
                    let mut outpath = file.mangled_name();

                    // If a common top-level folder exists, strip it.
                    if let Some(ref cp) = common_prefix {
                        if let Ok(stripped) = outpath.strip_prefix(cp) {
                            outpath = stripped.to_path_buf();
                        }
                    }

                    // Build the final output path under the extraction directory.
                    let final_path = extraction_dir.join(&outpath);

                    if file.name().ends_with('/') {
                        // Create directory entries, but do NOT add to extracted_files.
                        std::fs::create_dir_all(&final_path)?;
                    } else {
                        if let Some(parent) = final_path.parent() {
                            std::fs::create_dir_all(parent)?;
                        }
                        let mut outfile = std::fs::File::create(&final_path)?;
                        std::io::copy(&mut file, &mut outfile)?;
                        // Only add file paths (not directories) to the response.
                        if let Ok(relative) = final_path.strip_prefix(&output_dir_clone) {
                            extracted_files.push(relative.to_path_buf());
                        }
                    }
                }

                Ok(extracted_files)
            },
        )
        .await;

        // Clean up the temporary file.
        let _ = tokio::fs::remove_file(&temp_path).await;

        match extract_result {
            Ok(Ok(files)) => HttpResponse::Ok().json(Uploaded {
                message: format!("Zip file extracted {} files successfully", files.len()),
                paths: files,
                size: form.file.size,
            }),
            Ok(Err(e)) => {
                error!("Error extracting zip file: {:?}", e);
                HttpResponse::InternalServerError().json(json!({
                    "error": "Failed to extract zip archive",
                    "details": e.to_string()
                }))
            }
            Err(e) => {
                error!("Task error: {:?}", e);
                HttpResponse::InternalServerError().json(json!({
                    "error": "Internal server error",
                    "details": e.to_string()
                }))
            }
        }
    } else {
        // Handle non-zip file upload by copying it to the destination folder.
        match tokio::fs::copy(&temp_path, &dest_path).await {
            Ok(_) => {
                // Remove the temporary file after successful copy.
                if let Err(e) = tokio::fs::remove_file(&temp_path).await {
                    error!("Failed to clean up temporary file: {}", e);
                }
                let relative_path = dest_path.strip_prefix(&output_dir).unwrap_or(&dest_path).to_path_buf();
                HttpResponse::Ok().json(Uploaded {
                    message: "File uploaded successfully".to_string(),
                    paths: vec![relative_path],
                    size: form.file.size,
                })
            }
            Err(e) => {
                error!("Error copying file: {:?}", e);
                HttpResponse::InternalServerError().json(json!({
                    "error": "Failed to save uploaded file",
                    "details": e.to_string()
                }))
            }
        }
    }
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
struct UploadConfig {
    /// Maximum total file size in bytes
    max_file_size: usize,
    /// Maximum memory buffer size in bytes
    max_memory_buffer: usize,
}

#[utoipa::path(
    options,
    path = "/upload",
    description = "Get upload configuration limits.",
    responses(
        (status = 200, description = "Upload configuration with size limits in bytes", body = UploadConfig, content_type = "application/json", examples(
            ("default" = (summary = "Default configuration", value = json!({
                "max_file_size": UPLOAD_TOTAL_LIMIT,
                "max_memory_buffer": UPLOAD_MEMORY_LIMIT
            })))
        )),
    )
)]
async fn upload_config() -> impl Responder {
    let config = UploadConfig::default();
    HttpResponse::Ok().append_header(("Content-Type", "application/json")).json(config)
}

#[utoipa::path(
    post,
    path = "/index",
    description =   "Index content from various sources (files, texts, or images).</br>
                    If `source` is not specified, it will default to `/global`.",
    request_body(content = IndexRequest, examples(
        ("globs" = (summary = "Index files using glob patterns", value = json!({
            "source": "/bioma",
            "globs": ["./path/to/files/**/*.rs"], 
            "chunk_capacity": {"start": 500, "end": 2000},
            "chunk_overlap": 200,
            "chunk_batch_size": 50,
            "summarize": false
        }))),
        ("texts" = (summary = "Index text content directly", value = json!({
            "source": "/bioma",
            "texts": ["This is some text to index", "Here is another text"],
            "mime_type": "text/plain",
            "chunk_capacity": {"start": 500, "end": 2000},
            "chunk_overlap": 200,
            "chunk_batch_size": 50,
            "summarize": false
        }))),
        ("images" = (summary = "Index base64 encoded images", value = json!({
            "source": "/bioma",
            "images": ["base64_encoded_image_data"],
            "mime_type": "image/jpeg",
            "summarize": false
        })))
    )),
    responses(
        (status = 200, description = "Content indexed successfully", body = Indexed, content_type = "application/json", examples(
            ("globs_success" = (summary = "Successfully indexed files", value = json!({
                "indexed": 5,
                "cached": 2,
                "sources": [
                    {
                        "source": "/bioma",
                        "uri": "./path/to/files/main.rs",
                        "status": "Indexed"
                    },
                    {
                        "source": "/bioma",
                        "uri": "./path/to/files/lib.rs",
                        "status": "Cached"
                    },
                    {
                        "source": "/bioma",
                        "uri": "./path/to/files/error.rs",
                        "status": { "Failed": "File not found" }
                    }
                ]
            }))),
            ("texts_success" = (summary = "Successfully indexed texts", value = json!({
                "indexed": 2,
                "cached": 0,
                "sources": [
                    {
                        "source": "/bioma",
                        "uri": "text_1",
                        "status": "Indexed"
                    },
                    {
                        "source": "/bioma",
                        "uri": "text_2",
                        "status": "Indexed"
                    }
                ]
            })))
        )),
    )
)]
async fn index(body: web::Json<IndexRequest>, data: web::Data<AppState>) -> HttpResponse {
    let user_actor = match data.user_actor().await {
        Ok(actor) => actor,
        Err(e) => return HttpResponse::InternalServerError().body(e.to_string()),
    };

    let index_request = body.clone().into();

    info!("Sending message to indexer actor");
    let response = user_actor
        .send_and_wait_reply::<Indexer, Index>(
            index_request,
            &data.indexer,
            SendOptions::builder().timeout(Duration::from_secs(600)).build(),
        )
        .await;
    match response {
        Ok(indexed) => {
            info!("Indexed {} files, cached {} files", indexed.indexed, indexed.cached);
            HttpResponse::Ok().json(indexed)
        }
        Err(e) => {
            error!("Indexing error: {}", e);
            HttpResponse::InternalServerError().body(e.to_string())
        }
    }
}

#[utoipa::path(
    post,
    path = "/retrieve",
    description = "Retrieve context in .md format. If you send `sources` it will only use those resources to retrieve the context, if not, it will default to `/global`.",
    request_body(content = RetrieveContextRequest, examples(
        ("basic" = (summary = "Basic", value = json!({
            "type": "Text",
            "query": "What is Bioma?",
            "threshold": 0.0,
            "limit": 10,
            "sources": ["path/to/source1", "path/to/source2"],
            "format": "markdown"
        }))),
        ("without_sources" = (summary = "Without sources", value = json!({
            "type": "Text",
            "query": "What is Bioma?",
            "threshold": 0.0,
            "limit": 10,
            "format": "markdown"
        })))
    )),
    responses(
        (status = 200, description = "Retrieved context in the requested format (markdown or JSON)", body = String, content_type = "application/json"),
        (status = 500, description = "Internal server error")
    )
)]
async fn retrieve(body: web::Json<RetrieveContextRequest>, data: web::Data<AppState>) -> HttpResponse {
    let user_actor = match data.user_actor().await {
        Ok(actor) => actor,
        Err(e) => return HttpResponse::InternalServerError().body(e.to_string()),
    };

    let retrieve_context_request = body.clone().into();

    info!("Sending message to retriever actor");
    let response = user_actor
        .send_and_wait_reply::<Retriever, RetrieveContext>(
            retrieve_context_request,
            &data.retriever,
            SendOptions::builder().timeout(std::time::Duration::from_secs(200)).build(),
        )
        .await;

    match response {
        Ok(context) => {
            info!("Context fetched: {:#?}", context);
            let context_content = match body.format {
                RetrieveOutputFormat::Markdown => context.to_markdown(),
                RetrieveOutputFormat::Json => context.to_json(),
            };
            HttpResponse::Ok().json(context_content)
        }
        Err(e) => {
            error!("Error fetching context: {:?}", e);
            HttpResponse::InternalServerError().body(e.to_string())
        }
    }
}

#[utoipa::path(
    post,
    path = "/chat",
    description =   "For the usage of this endpoint, and more specifically, for the `tools` and `tools_actor` field, <b>only send of the two</b>.<br>
                    If you send `tools`, the definitions that you send, wil be send to the model and will return the tool call, without execute them.<br>
                    If you send `tools_actors`, the endpoint with call the client that contains the tools and actually execute them.<br>
                    If you send `sources` it will <b>only use those resources</b> to retrieve the context, if not, it will default to `/global`.",
    request_body(content = ChatQueryRequest, examples(
        ("Message only" = (summary = "Basic query", value = json!({
            "model": "llama3.2",
            "messages": [{"role": "user", "content": "Why is the sky blue?"}]
        }))),
        ("Message with_sources" = (summary = "With sources", value = json!({
            "model": "llama3.2",
            "messages": [{"role": "user", "content": "Why is the sky blue?"}],
            "sources": ["path/to/source1", "path/to/source2"]
        }))),
        ("with_tools" = (summary = "Using the echo tool as en example", value = json!({
            "messages": [
                {
                    "role": "user",
                    "content": "Why is the sky blue?"
                }
            ],
            "stream": true,
            "tools": [
                {
                    "function": {
                        "description": "Echoes back the input message",
                        "name": "echo",
                        "parameters": {
                            "message": "Echo this message"
                        }
                    },
                    "type": "function"
                }
            ]
        }))),
        ("with_multiple_tools" = (summary = "Sending multiple tools in payload", value = json!({
            "messages": [
                {
                    "role": "user",
                    "content": "Please generate a random number, start 15, end 1566. Then, echo that number and write the result to a file called path/to/file/test.txt"
                }
            ],
            "stream": false,
            "tools": [
                {
                    "function": {
                        "description": "Echoes back the input message",
                        "name": "echo",
                        "parameters": {
                            "message": "Echo this message"
                        }
                    },
                    "type": "function"
                },
                {
                    "function": {
                        "description": "Generate a random number",
                        "name": "random",
                        "parameters": {
                            "start": 12,
                            "end": 150
                        }
                    },
                    "type": "function"
                },
                {
                    "function": {
                        "description": "Create a new file or completely overwrite an existing file with new content. Use with caution as it will overwrite existing files without warning. Handles text content with proper encoding. Only works within allowed directories.",
                        "name": "write_file",
                        "parameters": {
                            "path": "/path/to/file",
                            "content": "writing into file"
                        }
                    },
                    "type": "function"
                },
                {
                    "function": {
                        "description": "Read the complete contents of a file from the file system. Handles various text encodings and provides detailed error messages if the file cannot be read. Use this tool when you need to examine the contents of a single file. Only works within allowed directories.",
                        "name": "read_file",
                        "parameters": {
                            "path": "/path/to/file"
                        }
                    },
                    "type": "function"
                }
            ]
        }))),
        ("with_tools_actors" = (summary = "Sending tools actor in payload", value = json!({
            "messages": [
                {
                    "role": "user",
                    "content": "Please generate a random number, start 15, end 1566. Then, echo that number and write the result to a file called path/to/file/test.txt"
                }
            ],
            "stream": true,
            "tools_actors":  ["/rag/tools_hub/your_id"]
        })))
    )),
    responses(
        (status = 200, description = "Chat response", body = ChatResponse, content_type = "application/json", examples(
            ("basic_response" = (summary = "Basic response", value = json!({
                "model": "llama2",
                "created_at": "2024-03-14T10:30:00Z",
                "message": {
                    "role": "assistant",
                    "content": "The sky appears blue due to a phenomenon called Rayleigh scattering..."
                },
                "done": true,
                "final_data": {
                    "total_duration": 1200000000,
                    "prompt_eval_count": 24,
                    "prompt_eval_duration": 500000000,
                    "eval_count": 150,
                    "eval_duration": 700000000
                },
                "context": [
                    {
                        "role": "user",
                        "content": "Why is the sky blue?"
                    }
                ]
            })))
        )),
        (status = 500, description = "Internal server error")
    )
)]
async fn chat(body: web::Json<ChatQueryRequest>, data: web::Data<AppState>) -> HttpResponse {
    let user_actor = match data.user_actor().await {
        Ok(actor) => actor,
        Err(e) => return HttpResponse::InternalServerError().body(e.to_string()),
    };

    let body = body.clone();

    // Combine all user messages into a single query string for the retrieval system.
    // This helps find relevant context across all parts of the user's conversation.
    let query = {
        // Extract only the user messages, ignoring system/assistant messages
        let user_messages: Vec<_> = body.messages.iter().filter(|message| message.role == MessageRole::User).collect();

        // Pre-allocate string capacity to avoid reallocations
        // Size = sum of all message lengths + newlines between messages
        let total_len: usize = user_messages.iter().map(|msg| msg.content.len()).sum();
        let mut result = String::with_capacity(total_len + user_messages.len());

        // Join all user messages with newlines to create a single search query
        for (i, message) in user_messages.iter().enumerate() {
            if i > 0 {
                result.push('\n');
            }
            result.push_str(&message.content);
        }
        result
    };

    // Retrieve relevant context based on the user's query
    let retrieve_context = RetrieveContext {
        query: RetrieveQuery::Text(query.clone()),
        limit: data.config.retrieve_limit,
        threshold: 0.0,
        sources: body.sources.clone(),
    };

    let context = user_actor
        .send_and_wait_reply::<Retriever, RetrieveContext>(
            retrieve_context,
            &data.retriever,
            SendOptions::builder().timeout(std::time::Duration::from_secs(200)).build(),
        )
        .await;

    match context {
        Ok(mut context) => {
            // Reverse to keep more relevant context near user query
            context.reverse();
            info!("Context fetched: {:#?}", context);

            // Preserve user-provided system message if present, otherwise use default (our own system prompt)
            let (system_message, filtered_messages): (Option<ChatMessage>, Vec<_>) = {
                let mut sys_msg = None;
                let filtered = if !body.messages.is_empty() {
                    body.messages[..body.messages.len() - 1]
                        .iter()
                        .filter(|msg| {
                            if msg.role == MessageRole::System {
                                sys_msg = Some((*msg).clone());
                                false
                            } else {
                                true
                            }
                        })
                        .cloned()
                        .collect()
                } else {
                    Vec::new()
                };
                (sys_msg, filtered)
            };

            // Create the system message with context
            let mut context_message = if let Some(sys_msg) = system_message {
                // Use the request's system message and append context
                ChatMessage::system(format!(
                    "Use the following context to answer the user's query:\n{}\n\n{}",
                    context.to_markdown(),
                    sys_msg.content
                ))
            } else {
                // Use our own system prompt
                ChatMessage::system(format!("{}{}", data.config.chat_prompt, context.to_markdown()))
            };

            // Add image handling here
            if let Some(ctx) = context
                .context
                .iter()
                .find(|ctx| ctx.metadata.as_ref().map_or(false, |m| matches!(m, Metadata::Image(_))))
            {
                if let (Some(source), Some(metadata)) = (&ctx.source, &ctx.metadata) {
                    match &metadata {
                        Metadata::Image(_image_metadata) => match tokio::fs::read(&source.uri).await {
                            Ok(image_data) => {
                                match tokio::task::spawn_blocking(move || {
                                    base64::engine::general_purpose::STANDARD.encode(image_data)
                                })
                                .await
                                {
                                    Ok(base64_data) => {
                                        context_message.images = Some(vec![Image::from_base64(&base64_data)]);
                                    }
                                    Err(e) => {
                                        error!("Error encoding image: {:?}", e);
                                    }
                                }
                            }
                            Err(e) => {
                                error!("Error reading image file: {:?}", e);
                            }
                        },
                        _ => {}
                    }
                }
            }

            // Set up streaming channel with sufficient buffer for chat responses
            let (tx, rx) = tokio::sync::mpsc::channel(1000);

            // Spawn a task to handle the chat stream processing
            tokio::spawn(async move {
                let mut conversation = Vec::with_capacity(filtered_messages.len() + 2);

                // Build conversation with messages if present
                if !body.messages.is_empty() {
                    conversation.extend(filtered_messages);
                    conversation.push(context_message.clone());
                    // Only add last message if it exists
                    if let Some(last_message) = body.messages.last() {
                        conversation.push(last_message.clone());
                    }
                } else {
                    // If no messages, just use the context message
                    conversation.push(context_message.clone());
                }

                // If caller provided tools in the request body, then we don't know how to execute them
                // so we assume the caller knows what they are doing and we just generate tool calls
                let client_tools = body.tools.clone();

                if !client_tools.is_empty() {
                    let chat_response = user_actor
                        .send_and_wait_reply::<Chat, ChatMessages>(
                            ChatMessages {
                                messages: conversation.clone(),
                                restart: true,
                                persist: false,
                                stream: false,
                                format: body.format.clone(),
                                tools: Some(client_tools.into_iter().map(Into::into).collect()),
                            },
                            &data.chat,
                            SendOptions::builder().timeout(std::time::Duration::from_secs(600)).build(),
                        )
                        .await;

                    match chat_response {
                        Ok(response) => {
                            let response = ChatResponse { response, context: conversation.clone() };
                            let _ = tx.send(Ok(Json(response))).await;
                            return Ok(());
                        }
                        Err(e) => {
                            error!("Error during chat with tools: {:?}", e);
                            let _ = tx.send(Err(e.to_string())).await;
                            return Err(cognition::ChatToolError::FetchToolsError(e.to_string()));
                        }
                    }
                }

                let mut tools: Vec<ToolInfo> = vec![];
                let mut tool_hub_map: ToolHubMap = HashMap::new();

                for actor_id in &body.tools_actors {
                    let actor_id = ActorId::of::<ToolsHub>(actor_id.clone());
                    let tool_info = user_actor
                        .send_and_wait_reply::<ToolsHub, ListTools>(ListTools(None), &actor_id, SendOptions::default())
                        .await;
                    match tool_info {
                        Ok(tool_info) => {
                            // TODO: What if we have functions with the same name?
                            // Map each tool name to this hub's ID
                            for tool in &tool_info {
                                tool_hub_map.insert(tool.name().to_string(), actor_id.clone());
                            }
                            tools.extend(tool_info);
                        }
                        Err(e) => {
                            error!("Error fetching tools: {:?}", e);
                            let _ = tx.send(Err(e.to_string())).await;
                            continue;
                        }
                    }
                }

                for tool_info in &tools {
                    info!("Tool: {}", tool_info.name());
                }

                let chat_with_tools_tx = tx.clone();
                let tool_call_tree = cognition::chat_with_tools(
                    &user_actor,
                    &data.chat,
                    &conversation,
                    &tools,
                    &tool_hub_map,
                    chat_with_tools_tx,
                    body.format.clone(),
                    body.stream,
                )
                .await;

                if let Err(e) = tool_call_tree {
                    if tools.len() > 0 {
                        error!("Error during chat with tools: {:?}", e);
                    } else {
                        error!("Error during chat: {:?}", e);
                    }
                }

                Ok(())
            });

            // Stream responses as NDJSON
            HttpResponse::Ok().content_type("application/x-ndjson").streaming::<_, Box<dyn StdError>>(
                tokio_stream::wrappers::ReceiverStream::new(rx).map(|result| match result {
                    Ok(response) => {
                        let json = serde_json::to_string(&response).unwrap_or_default();
                        Ok(web::Bytes::from(format!("{}\n", json)))
                    }
                    Err(e) => {
                        let error_json = serde_json::json!({
                            "error": e.to_string()
                        });
                        Ok(web::Bytes::from(format!("{}\n", error_json)))
                    }
                }),
            )
        }
        Err(e) => {
            error!("Error fetching context: {:?}", e);
            HttpResponse::InternalServerError().body(format!("Error fetching context: {}", e))
        }
    }
}

#[utoipa::path(
    post,
    path = "/think",
    description =   "Analyzes query and determines which tools to use and in what order.<br>
                    For the usage of this endpoint, and more specifically, for the `tools` and `tools_actors` fields, <b>only</b> send of the two.<br>
                    If you send `tools`, the definitions that you send, wil be send to the model and will return the tool call, <b>without execute them</b>.<br>
                    If you send `tools_actors`, the endpoint with call the client that contains the tools and return the tools information from all tools_actors.<br>
                    If you send 'sources' it will <b>only use those resources</b> to retrieve the context, if not, it will default to `/global`.",
    request_body(content = ThinkQueryRequest, examples(
        ("Message only" = (summary = "Basic query", value = json!({
            "messages": [
                {
                    "role": "user",
                    "content": "Why is the sky blue?"
                }
            ]
        }))),
        ("With sources" = (summary = "With sources", value = json!({
            "messages": [
                {
                    "role": "user",
                    "content": "Why is the sky blue?"
                }
            ],
            "sources": ["/bioma"]
        }))),
        ("with_tools" = (summary = "Using the echo tool as en example", value = json!({
            "messages": [
                {
                    "role": "user",
                    "content": "Why is the sky blue?"
                }
            ],
            "stream": false,
            "tools": [
                {
                    "function": {
                        "description": "Echoes back the input message",
                        "name": "echo",
                        "parameters": {
                            "message": "Echo this message"
                        }
                    },
                    "type": "function"
                }
            ]
        }))),
        ("with_multiple_tools" = (summary = "Sending multiple tools", value = json!({
            "messages": [
                {
                    "role": "user",
                    "content": "Please generate a random number, start 15, end 1566, then, echo that number and write it as content into to the file /path/to/file.txt"
                }
            ],
            "stream": false,
            "tools": [
                {
                    "function": {
                        "description": "Echoes back the input message",
                        "name": "echo",
                        "parameters": {
                            "message": "Echo this message"
                        }
                    },
                    "type": "function"
                },
                {
                    "function": {
                        "description": "Generate a random number",
                        "name": "random",
                        "parameters": {
                            "start": 12,
                            "end": 150
                        }
                    },
                    "type": "function"
                },
                {
                    "function": {
                        "description": "Create a new file or completely overwrite an existing file with new content. Use with caution as it will overwrite existing files without warning. Handles text content with proper encoding. Only works within allowed directories.",
                        "name": "write_file",
                        "parameters": {
                            "path": "/path/to/file",
                            "content": "writing into file"
                        }
                    },
                    "type": "function"
                },
                {
                    "function": {
                        "description": "Read the complete contents of a file from the file system. Handles various text encodings and provides detailed error messages if the file cannot be read. Use this tool when you need to examine the contents of a single file. Only works within allowed directories.",
                        "name": "read_file",
                        "parameters": {
                            "path": "/path/to/file"
                        }
                    },
                    "type": "function"
                },
            ]
        }))),
        ("with_tools_actors" = (summary = "Sending tools actor in payload", value = json!({
            "messages": [
                {
                    "role": "user",
                    "content": "Please generate a random number, start 15, end 1566. Then, echo that number and write the result to a file called path/to/file/test.txt"
                }
            ],
            "stream": true,
            "tools_actors":  ["/rag/tools_hub/your_id"]
        })))
    )),
    responses(
        (status = 200, description = "Chat response with tool analysis", body = ChatResponse, content_type = "application/json", examples(
            ("basic_response" = (summary = "Basic response", value = json!({
                "model": "llama2",
                "created_at": "2024-03-14T10:30:00Z",
                "message": {
                    "role": "assistant",
                    "content": "Based on your request, I'll help you analyze and determine the tools needed. Here's what we need to do:\n\n1. Use the `random` tool to generate a number between 15 and 1566\n2. Use the `echo` tool to display the generated number\n3. Use the `write_file` tool to save the number to the specified file"
                },
                "done": true,
                "final_data": {
                    "total_duration": 1200000000,
                    "prompt_eval_count": 24,
                    "prompt_eval_duration": 500000000,
                    "eval_count": 150,
                    "eval_duration": 700000000
                },
                "context": [
                    {
                        "role": "user",
                        "content": "Please generate a random number, start 15, end 1566. Then, echo that number and write the result to a file called path/to/file/test.txt"
                    }
                ]
            })))
        )),
        (status = 500, description = "Internal server error")
    )
)]
async fn think(body: web::Json<ThinkQueryRequest>, data: web::Data<AppState>) -> HttpResponse {
    let user_actor = match data.user_actor().await {
        Ok(actor) => actor,
        Err(e) => return HttpResponse::InternalServerError().body(e.to_string()),
    };

    let query = body
        .messages
        .iter()
        .filter(|message| message.role == MessageRole::User)
        .map(|message| message.content.clone())
        .collect::<Vec<String>>()
        .join("\n");

    let retrieve_context = RetrieveContext {
        query: RetrieveQuery::Text(query.clone()),
        limit: data.config.retrieve_limit,
        threshold: 0.0,
        sources: body.sources.clone(),
    };

    let retrieved = match user_actor
        .send_and_wait_reply::<Retriever, RetrieveContext>(
            retrieve_context,
            &data.retriever,
            SendOptions::builder().timeout(std::time::Duration::from_secs(200)).build(),
        )
        .await
    {
        Ok(context) => context,
        Err(e) => return HttpResponse::InternalServerError().body(format!("Error fetching context: {}", e)),
    };

    let context_content = if retrieved.context.is_empty() {
        "No additional context available".to_string()
    } else {
        retrieved.to_markdown()
    };

    // Preserve user-provided system message if present, otherwise use default (our own system prompt)
    let (system_message, filtered_messages): (Option<ChatMessage>, Vec<_>) = {
        let mut sys_msg = None;
        let filtered = if !body.messages.is_empty() {
            body.messages[..body.messages.len() - 1]
                .iter()
                .filter(|msg| {
                    if msg.role == MessageRole::System {
                        sys_msg = Some((*msg).clone());
                        false
                    } else {
                        true
                    }
                })
                .cloned()
                .collect()
        } else {
            Vec::new()
        };
        (sys_msg, filtered)
    };

    // Build the system prompt based on system message or tools
    let system_prompt = if let Some(sys_msg) = system_message {
        // If system message exists, use it directly with context
        format!("Use the following context to answer the user's query:\n{}\n\n{}", context_content, sys_msg.content)
    } else if !body.tools.is_empty() {
        // If tools are provided in the request, use those directly
        let tools_str = body
            .tools
            .iter()
            .map(|t| {
                let params = serde_json::to_string_pretty(&t.function().parameters)
                    .unwrap_or_else(|_| "Unable to parse parameters".to_string());
                format!(
                    "- {}: {}\n  Parameters:\n{}",
                    t.function().name,
                    t.function().description,
                    params.split('\n').map(|line| format!("    {}", line)).collect::<Vec<_>>().join("\n")
                )
            })
            .collect::<Vec<_>>()
            .join("\n\n");

        format!(
            "{}\n\nADDITIONAL CONTEXT:\n{}",
            data.config.tool_prompt.replace("{tools_list}", &tools_str),
            context_content
        )
    } else {
        // Get tools from tools_actors like in chat endpoint
        let mut tools: Vec<ToolInfo> = vec![];
        let mut tool_hub_map: ToolHubMap = HashMap::new();

        // First collect all tools from all actors
        for actor_id in &body.tools_actors {
            let actor_id = ActorId::of::<ToolsHub>(actor_id.clone());
            match user_actor
                .send_and_wait_reply::<ToolsHub, ListTools>(ListTools(None), &actor_id, SendOptions::default())
                .await
            {
                Ok(tool_info) => {
                    // Map each tool name to this hub's ID
                    for tool in &tool_info {
                        tool_hub_map.insert(tool.name().to_string(), actor_id.clone());
                    }
                    tools.extend(tool_info);
                }
                Err(e) => {
                    error!("Error fetching tools: {:?}", e);
                    return HttpResponse::InternalServerError().body(format!("Error fetching tools: {}", e));
                }
            }
        }

        // Then build the tools string once we have all tools
        if !tools.is_empty() {
            let tools_str = tools
                .iter()
                .map(|t| {
                    let params = serde_json::to_string_pretty(t.parameters())
                        .unwrap_or_else(|_| "Unable to parse parameters".to_string());
                    format!(
                        "- {}: {}\n  Parameters:\n{}",
                        t.name(),
                        t.description(),
                        params.split('\n').map(|line| format!("    {}", line)).collect::<Vec<_>>().join("\n")
                    )
                })
                .collect::<Vec<_>>()
                .join("\n\n");

            format!(
                "{}\n\nADDITIONAL CONTEXT:\n{}",
                data.config.tool_prompt.replace("{tools_list}", &tools_str),
                context_content
            )
        } else {
            // Default system prompt if no tools were found
            format!("{}\n\nADDITIONAL CONTEXT:\n{}", data.config.chat_prompt, context_content)
        }
    };

    let mut context_message = ChatMessage::system(system_prompt);

    if let Some(ctx) =
        retrieved.context.iter().find(|ctx| ctx.metadata.as_ref().map_or(false, |m| matches!(m, Metadata::Image(_))))
    {
        if let (Some(source), Some(metadata)) = (&ctx.source, &ctx.metadata) {
            if let Metadata::Image(_) = metadata {
                if let Ok(image_data) = tokio::fs::read(&source.uri).await {
                    if let Ok(base64_data) = tokio::task::spawn_blocking(move || {
                        base64::engine::general_purpose::STANDARD.encode(image_data)
                    })
                    .await
                    {
                        context_message.images = Some(vec![Image::from_base64(&base64_data)]);
                    }
                }
            }
        }
    }

    let mut conversation = Vec::with_capacity(filtered_messages.len() + 2);
    if !body.messages.is_empty() {
        conversation.extend(filtered_messages);
        conversation.push(context_message.clone());
        conversation.push(body.messages[body.messages.len() - 1].clone());
    } else {
        conversation.push(context_message.clone());
    }

    // Set up streaming channel
    let (tx, rx) = tokio::sync::mpsc::channel(1000);

    // Spawn task to handle chat processing
    tokio::spawn(async move {
        let chat_request = ChatMessages {
            messages: conversation.clone(),
            restart: true,
            persist: false,
            stream: body.stream.clone(),
            format: body.format.clone(),
            tools: None,
        };

        let mut chat_response = match user_actor
            .send::<Chat, ChatMessages>(
                chat_request,
                &data.think_chat,
                SendOptions::builder().timeout(std::time::Duration::from_secs(600)).build(),
            )
            .await
        {
            Ok(stream) => stream,
            Err(e) => {
                let _ = tx.send(Err(e.to_string())).await;
                return;
            }
        };

        // Stream the responses
        let mut is_first_message = true;
        while let Some(response) = chat_response.next().await {
            match response {
                Ok(message_response) => {
                    // Create response with context only on first message
                    let response = ChatResponse {
                        response: message_response,
                        context: if is_first_message { conversation.clone() } else { vec![] },
                    };
                    is_first_message = false;

                    if tx.send(Ok(Json(response))).await.is_err() {
                        break;
                    }
                }
                Err(e) => {
                    let _ = tx.send(Err(e.to_string())).await;
                    break;
                }
            }
        }
    });

    // Stream responses as NDJSON
    HttpResponse::Ok().content_type("application/x-ndjson").streaming::<_, Box<dyn StdError>>(
        tokio_stream::wrappers::ReceiverStream::new(rx).map(|result| match result {
            Ok(response) => {
                let json = serde_json::to_string(&response).unwrap_or_default();
                Ok(web::Bytes::from(format!("{}\n", json)))
            }
            Err(e) => {
                let error_json = serde_json::json!({
                    "error": e.to_string()
                });
                Ok(web::Bytes::from(format!("{}\n", error_json)))
            }
        }),
    )
}

#[derive(Serialize, utoipa::ToSchema)]
struct AskResponse {
    #[serde(flatten)]
    response: ChatMessageResponse,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    context: Vec<ChatMessage>,
}

#[utoipa::path(
    post,
    path = "/ask",
    description = "Generates a structured chat response based on a specific format schema. This endpoint is designed for getting formatted, structured responses rather than free-form chat.\nIf you send `sources` it will only use those resources to retrieve the context, if not, it will default to `/global`.",
    request_body(content = AskQueryRequest, examples(
        ("Basic" = (summary = "Basic structured query", value = json!({
            "messages": [
                {
                    "role": "user",
                    "content": "Tell me about Puerto Rico."
                }
            ],
            "format": {
                "title": "PuertoRicoInfo",
                "type": "object",
                "required": [
                    "name",
                    "capital",
                    "languages"
                ],
                "properties": {
                    "name": {
                        "description": "Name of the territory",
                        "type": "string"
                    },
                    "capital": {
                        "description": "Capital city",
                        "type": "string"
                    },
                    "languages": {
                        "description": "Official languages spoken",
                        "type": "array",
                        "items": {
                            "type": "string"
                        }
                    }
                }
            }
        }))),
        ("With sources" = (summary = "Query with specific sources", value = json!({
            "messages": [
                {
                    "role": "user",
                    "content": "Tell me about Puerto Rico."
                }
            ],
            "sources": ["/path/to/source1.pdf", "/path/to/source2.pdf"],
            "format": {
                "title": "PuertoRicoInfo",
                "type": "object",
                "required": [
                    "name",
                    "capital",
                    "languages"
                ],
                "properties": {
                    "name": {
                        "description": "Name of the territory",
                        "type": "string"
                    },
                    "capital": {
                        "description": "Capital city",
                        "type": "string"
                    },
                    "languages": {
                        "description": "Official languages spoken",
                        "type": "array",
                        "items": {
                            "type": "string"
                        }
                    }
                }
            }
        })))
    )),
    responses(
        (status = 200, description = "Structured chat response", body = AskResponse, content_type = "application/json", examples(
            ("basic_response" = (summary = "Structured response", value = json!({
                "model": "llama2",
                "created_at": "2024-03-14T10:30:00Z",
                "message": {
                    "role": "assistant",
                    "content": {
                        "name": "Puerto Rico",
                        "capital": "San Juan",
                        "languages": ["Spanish", "English"]
                    }
                },
                "done": true,
                "final_data": {
                    "total_duration": 1200000000,
                    "prompt_eval_count": 24,
                    "prompt_eval_duration": 500000000,
                    "eval_count": 150,
                    "eval_duration": 700000000
                },
                "context": [
                    {
                        "role": "user",
                        "content": "Tell me about Puerto Rico."
                    }
                ]
            })))
        )),
        (status = 500, description = "Internal server error")
    )
)]
async fn ask(body: web::Json<AskQueryRequest>, data: web::Data<AppState>) -> HttpResponse {
    let user_actor = match data.user_actor().await {
        Ok(actor) => actor,
        Err(e) => return HttpResponse::InternalServerError().body(e.to_string()),
    };

    let body = body.clone();

    let query = body
        .messages
        .iter()
        .filter(|message| message.role == MessageRole::User)
        .map(|message| message.content.clone())
        .collect::<Vec<String>>()
        .join("\n");
    info!("Received chat query: {:#?}", query);

    info!("Sending message to retriever actor");
    let retrieve_context = RetrieveContext {
        query: RetrieveQuery::Text(query.clone()),
        limit: data.config.retrieve_limit,
        threshold: 0.0,
        sources: body.sources.clone(),
    };

    let retrieved = user_actor
        .send_and_wait_reply::<Retriever, RetrieveContext>(
            retrieve_context,
            &data.retriever,
            SendOptions::builder().timeout(std::time::Duration::from_secs(200)).build(),
        )
        .await;

    match retrieved {
        Ok(context) => {
            info!("Context fetched: {:#?}", context);
            let context_content = context.to_markdown();

            // Preserve user-provided system message if present, otherwise use default (our own system prompt)
            let (system_message, filtered_messages): (Option<ChatMessage>, Vec<_>) = {
                let mut sys_msg = None;
                let filtered = if !body.messages.is_empty() {
                    body.messages[..body.messages.len() - 1]
                        .iter()
                        .filter(|msg| {
                            if msg.role == MessageRole::System {
                                sys_msg = Some((*msg).clone());
                                false
                            } else {
                                true
                            }
                        })
                        .cloned()
                        .collect()
                } else {
                    Vec::new()
                };
                (sys_msg, filtered)
            };

            // Create the system message with context
            let mut context_message = if let Some(sys_msg) = system_message {
                // Use existing system message and append context
                ChatMessage::system(format!(
                    "Use the following context to answer the user's query:\n{}\n\n{}",
                    context_content, sys_msg.content
                ))
            } else {
                // Use default prompt
                ChatMessage::system(format!("{}{}", data.config.chat_prompt, context_content))
            };

            // Handle image context if present
            if let Some(ctx) = context
                .context
                .iter()
                .find(|ctx| ctx.metadata.as_ref().map_or(false, |m| matches!(m, Metadata::Image(_))))
            {
                if let (Some(source), Some(metadata)) = (&ctx.source, &ctx.metadata) {
                    match &metadata {
                        Metadata::Image(_image_metadata) => match tokio::fs::read(&source.uri).await {
                            Ok(image_data) => {
                                match tokio::task::spawn_blocking(move || {
                                    base64::engine::general_purpose::STANDARD.encode(image_data)
                                })
                                .await
                                {
                                    Ok(base64_data) => {
                                        context_message.images = Some(vec![Image::from_base64(&base64_data)]);
                                    }
                                    Err(e) => {
                                        error!("Error encoding image: {:?}", e);
                                    }
                                }
                            }
                            Err(e) => {
                                error!("Error reading image file: {:?}", e);
                            }
                        },
                        _ => {}
                    }
                }
            }

            let mut conversation = Vec::with_capacity(filtered_messages.len() + 2);
            if !body.messages.is_empty() {
                conversation.extend(filtered_messages);
                conversation.push(context_message.clone());
                conversation.push(body.messages[body.messages.len() - 1].clone());
            } else {
                conversation.push(context_message.clone());
            }

            info!("Sending context to chat actor");
            let ask_response = user_actor
                .send_and_wait_reply::<Chat, ChatMessages>(
                    ChatMessages {
                        messages: conversation.clone(),
                        restart: true,
                        persist: false,
                        stream: false,
                        format: body.format.clone(),
                        tools: None,
                    },
                    &data.chat,
                    SendOptions::builder().timeout(std::time::Duration::from_secs(600)).build(),
                )
                .await;

            match ask_response {
                Ok(response) => {
                    info!("Ask response: {:#?}", &response);
                    HttpResponse::Ok().json(AskResponse { response, context: conversation })
                }
                Err(e) => {
                    error!("Error fetching ask response: {:?}", e);
                    HttpResponse::InternalServerError().body(format!("Error fetching ask response: {}", e))
                }
            }
        }
        Err(e) => {
            error!("Error fetching context: {:?}", e);
            HttpResponse::InternalServerError().body(format!("Error fetching context: {}", e))
        }
    }
}

#[utoipa::path(
    post,
    path = "/delete_source",
    description = "Delete indexed sources and their associated embeddings from the system.",
    request_body(content = DeleteSource, examples(
        ("Basic" = (summary = "Delete multiple sources", value = json!({
            "sources": [
                "/path/to/source1.pdf",
                "/path/to/source2.md",
                "/path/to/directory/*"
            ]
        })))
    )),
    responses(
        (status = 200, description = "Sources deleted successfully", body = DeletedSource, content_type = "application/json", examples(
            ("success" = (summary = "Successful deletion", value = json!({
                "deleted_embeddings": 42,
                "deleted_sources": [
                    {
                        "source": "/global",
                        "uri": "/path/to/source1.pdf"
                    },
                    {
                        "source": "/global",
                        "uri": "/path/to/source2.md"
                    }
                ]
            })))
        )),
        (status = 500, description = "Internal server error")
    )
)]
async fn delete_source(body: web::Json<DeleteSource>, data: web::Data<AppState>) -> HttpResponse {
    let user_actor = match data.user_actor().await {
        Ok(actor) => actor,
        Err(e) => return HttpResponse::InternalServerError().body(e.to_string()),
    };

    let delete_source = body.clone().into();

    info!("Sending delete message to indexer actor for sources: {:?}", body.sources);
    let response = user_actor
        .send_and_wait_reply::<Indexer, DeleteSource>(
            delete_source,
            &data.indexer,
            SendOptions::builder().timeout(std::time::Duration::from_secs(200)).build(),
        )
        .await;

    match response {
        Ok(result) => {
            info!(
                "Deleted {} embeddings. Successfully deleted sources: {:?}",
                result.deleted_embeddings, result.deleted_sources
            );
            HttpResponse::Ok().json(result)
        }
        Err(e) => {
            error!("Error deleting sources: {:?}", e);
            HttpResponse::InternalServerError().body(e.to_string())
        }
    }
}

#[utoipa::path(
    post,
    path = "/embed",
    description = "Generate embeddings for text or images. Supports both single inputs and batches.\nFor text, accepts plain text or arrays of text.\nFor images, accepts base64-encoded image data or arrays of base64 images.",
    request_body(content = EmbeddingsQueryRequest, examples(
        ("Single Text" = (summary = "Single text input", value = json!({
            "model": "nomic-embed-text",
            "input": "This text will generate embeddings"
        }))),
        ("Multiple Texts" = (summary = "Multiple text inputs", value = json!({
            "model": "nomic-embed-text",
            "input": [
                "First text to embed",
                "Second text to embed",
                "Third text to embed"
            ]
        }))),
        ("Single Image" = (summary = "Single image input", value = json!({
            "model": "nomic-embed-vision",
            "input": "iVBORw0KGgoAAAANSUhEUgAAAAoAAAAKCAIAAAACUFjqAAABRklEQVR4nAA2Acn+A2ql2+Vv1LF7X3Mw2i9cMEBUs0/l0C6/irfF6wPqowTw0ORE00EZ/He1x+LwZ3nDwaZVNIgn6FI8KQabKikArD0j4g6LU2Mz9DpsAgnYGy6195whWQQ4XIk1a74tA98BtQfyE3oQkaA/uufBkIegK+TH6LMh/O44hIio5wAw4umxtkxZNCIf35A4YNshDwNeeHFnHP0YUSelrm8DMioFvjc7QOcZmEBw/pv+SXEH2G+O0ZdiHDTb6wnhAcRk1rkuJLwy/d7DDKTgqOflV5zk7IBgmz0f8J4o5gA4yb3rYzzUyLRXS0bY40xnoY/rtniWFdlrtSHkR/0A1ClG/qVWNyD1CXVkxE4IW5Tj+8qk1sD42XW6TQpPAO7NhmcDxDz092Q2AR8XYKPa1LPkGberOYArt0gkbQEAAP//4hWZNZ4Pc4kAAAAASUVORK5CYII="
        }))),
    )),
    responses(
        (status = 200, description = "Generated embeddings", body = GeneratedEmbeddings, content_type = "application/json", examples(
            ("text_embeddings" = (summary = "Text embeddings", value = json!({
                "embeddings": [
                    [0.1, 0.2, 0.3, 0.4, 0.5],
                    [0.2, 0.3, 0.4, 0.5, 0.6]
                ]
            })))
        )),
        (status = 400, description = "Invalid request (wrong model or input format)"),
        (status = 500, description = "Internal server error")
    )
)]
async fn embed(body: web::Json<EmbeddingsQueryRequest>, data: web::Data<AppState>) -> HttpResponse {
    let user_actor = match data.user_actor().await {
        Ok(actor) => actor,
        Err(e) => return HttpResponse::InternalServerError().body(e.to_string()),
    };

    // Process input based on model type
    let embedding_content = match body.model {
        ModelEmbed::NomicEmbedTextV15 => {
            // Handle text embeddings
            let texts = match body.input.as_str() {
                Some(s) => vec![s.to_string()],
                None => match body.input.as_array() {
                    Some(arr) => arr.iter().filter_map(|v| v.as_str().map(String::from)).collect(),
                    None => return HttpResponse::BadRequest().body("Input must be string or array of strings"),
                },
            };
            EmbeddingContent::Text(texts)
        }
        ModelEmbed::NomicEmbedVisionV15 => {
            // Handle base64 image embeddings
            let base64_images = match body.input.as_str() {
                Some(s) => vec![s.to_string()],
                None => match body.input.as_array() {
                    Some(arr) => arr.iter().filter_map(|v| v.as_str().map(String::from)).collect(),
                    None => return HttpResponse::BadRequest().body("Input must be string or array of strings"),
                },
            };

            // Validate base64 images
            let images = base64_images.into_iter().map(|base64| ImageData::Base64(base64)).collect();

            EmbeddingContent::Image(images)
        }
    };

    // Process embeddings - different handling for text vs images
    let mut all_embeddings = Vec::new();

    match &embedding_content {
        EmbeddingContent::Text(texts) => {
            // Process text in chunks
            const CHUNK_SIZE: usize = 10;
            for chunk in texts.chunks(CHUNK_SIZE) {
                match user_actor
                    .send_and_wait_reply::<Embeddings, GenerateEmbeddings>(
                        GenerateEmbeddings { content: EmbeddingContent::Text(chunk.to_vec()) },
                        &data.embeddings,
                        SendOptions::builder().timeout(std::time::Duration::from_secs(200)).build(),
                    )
                    .await
                {
                    Ok(chunk_response) => all_embeddings.extend(chunk_response.embeddings),
                    Err(e) => {
                        error!("Error processing text chunk: {:?}", e);
                        return HttpResponse::InternalServerError().body(e.to_string());
                    }
                }
            }
        }
        EmbeddingContent::Image(_) => {
            // Process all images in a single request
            match user_actor
                .send_and_wait_reply::<Embeddings, GenerateEmbeddings>(
                    GenerateEmbeddings { content: embedding_content },
                    &data.embeddings,
                    SendOptions::builder().timeout(std::time::Duration::from_secs(200)).build(),
                )
                .await
            {
                Ok(response) => all_embeddings.extend(response.embeddings),
                Err(e) => {
                    error!("Error processing images: {:?}", e);
                    return HttpResponse::InternalServerError().body(e.to_string());
                }
            }
        }
    }

    // Return combined results
    let generated_embeddings = GeneratedEmbeddings { embeddings: all_embeddings };
    HttpResponse::Ok().json(generated_embeddings)
}

#[utoipa::path(
    post,
    path = "/rerank",
    description = "Rerank a list of texts based on their relevance to a query. Can return raw similarity scores and optionally include the original texts in the response.",
    request_body(content = RankTexts, examples(
        ("Basic" = (summary = "Basic reranking", value = json!({
            "query": "What is Deep Learning?",
            "texts": [
                "Deep Learning is learning under water",
                "Deep learning is a branch of machine learning based on artificial neural networks",
                "Deep learning enables machines to learn from experience"
            ],
            "raw_scores": false,
            "return_text": true
        }))),
        ("Advanced" = (summary = "Advanced options", value = json!({
            "query": "What is Deep Learning?",
            "texts": [
                "Deep Learning is learning under water",
                "Deep learning is a branch of machine learning based on artificial neural networks",
                "Deep learning enables machines to learn from experience"
            ],
            "raw_scores": true,
            "return_text": true,
            "truncate": true,
            "truncation_direction": "right"
        })))
    )),
    responses(
        (status = 200, description = "Ranked texts", body = RankedTexts, content_type = "application/json", examples(
            ("with_text" = (summary = "Response with text included", value = json!({
                "texts": [
                    {
                        "index": 1,
                        "score": 0.92,
                        "text": "Deep learning is a branch of machine learning based on artificial neural networks"
                    },
                    {
                        "index": 2,
                        "score": 0.78,
                        "text": "Deep learning enables machines to learn from experience"
                    },
                    {
                        "index": 0,
                        "score": 0.45,
                        "text": "Deep Learning is learning under water"
                    }
                ]
            }))),
            ("without_text" = (summary = "Response without text", value = json!({
                "texts": [
                    {
                        "index": 1,
                        "score": 0.92
                    },
                    {
                        "index": 2,
                        "score": 0.78
                    },
                    {
                        "index": 0,
                        "score": 0.45
                    }
                ]
            })))
        )),
        (status = 500, description = "Internal server error")
    )
)]
async fn rerank(body: web::Json<RankTexts>, data: web::Data<AppState>) -> HttpResponse {
    let user_actor = match data.user_actor().await {
        Ok(actor) => actor,
        Err(e) => return HttpResponse::InternalServerError().body(e.to_string()),
    };

    let body = body.clone();

    let max_text_len = body.texts.iter().map(|text| text.len()).max().unwrap_or(0);
    info!("Received rerank query with {} texts (max. {} chars)", body.texts.len(), max_text_len);
    debug!("Rerank query: {}", body.query);

    let rank_texts = body.clone();

    match user_actor
        .send_and_wait_reply::<Rerank, RankTexts>(
            rank_texts,
            &data.rerank,
            SendOptions::builder().timeout(std::time::Duration::from_secs(200)).build(),
        )
        .await
    {
        Ok(ranked_texts) => {
            debug!("Reranked texts: {:#?}", ranked_texts);
            HttpResponse::Ok().json(ranked_texts)
        }
        Err(e) => {
            error!("Rerank error: {}", e);
            HttpResponse::InternalServerError().body(e.to_string())
        }
    }
}

#[utoipa::path(
    get,
    path = "/",
    description = "Serves the dashboard UI.",
    responses(
        (status = 200, description = "Ok"),
    )
)]
async fn dashboard() -> impl Responder {
    NamedFile::open_async("assets/dashboard.html").await
}

async fn swagger_initializer(data: web::Data<AppState>) -> impl Responder {
    // Get the base URL as a string, without trailing slash
    let endpoint = data.config.rag_endpoint.as_str().trim_end_matches('/');

    let js_content = format!(
        r#"window.onload = function() {{
  window.ui = SwaggerUIBundle({{
    url: "{}/api-docs/openapi.json",
    dom_id: '#swagger-ui',
    deepLinking: true,
    presets: [
      SwaggerUIBundle.presets.apis,
      SwaggerUIStandalonePreset
    ],
    plugins: [
      SwaggerUIBundle.plugins.DownloadUrl
    ],
    layout: "StandaloneLayout",
    syntaxHighlight: {{
      activated: true,
      theme: "monokai"
    }}
  }});
}};"#,
        endpoint
    );

    HttpResponse::Ok().content_type("application/javascript").body(js_content)
}

#[utoipa::path(
    get,
    path = "/sources",
    description = "List all indexed sources.",
    responses(
        (status = 200, description = "List of sources", body = ListedSources, content_type = "application/json", examples(
            ("Basic" = (summary = "Basic list of sources", value = json!({
                "sources": [
                    {
                        "source": "/global",
                        "uri": "/path/to/source1.pdf"
                    },
                    {
                        "source": "/global",
                        "uri": "/path/to/source2.md"
                    }
                ]
            })))
        )),
        (status = 500, description = "Internal server error")
    )
)]
async fn list_sources(data: web::Data<AppState>) -> HttpResponse {
    let user_actor = match data.user_actor().await {
        Ok(actor) => actor,
        Err(e) => return HttpResponse::InternalServerError().body(e.to_string()),
    };

    info!("Fetching list of sources");
    let response = user_actor
        .send_and_wait_reply::<Retriever, ListSources>(ListSources, &data.retriever, SendOptions::default())
        .await;

    match response {
        Ok(sources) => HttpResponse::Ok().json(sources),
        Err(e) => {
            error!("Error fetching sources: {:?}", e);
            HttpResponse::InternalServerError().body(e.to_string())
        }
    }
}

#[derive(OpenApi)]
#[openapi(
    paths(
        health,
        hello,
        reset,
        index,
        retrieve,
        ask,
        chat,
        think,
        upload,
        upload_config,
        delete_source,
        embed,
        rerank,
        dashboard,
        list_sources
    ),
    info(
        title = "Cognition API",
        version = "0.1.0",
        description = "API for the Cognition RAG system",
        license(name = "MIT", identifier = "MIT")
    )
)]
struct ApiDoc;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Initialize tracing
    let filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info"));
    tracing_subscriber::fmt().with_env_filter(filter).init();

    let args = Args::parse();
    let config = args.load_config()?;

    // Initialize engine
    let engine = Engine::connect(config.engine.clone()).await?;

    // Spawn main actors and their relays
    let mut actor_handles = Vec::new();

    // Indexer setup
    let indexer_id = ActorId::of::<Indexer>("/rag/indexer");
    let mut indexer = Indexer::default();
    indexer.summary = Summary::builder()
        .chat(Chat::builder().model(config.chat_model.clone()).endpoint(config.chat_endpoint.clone()).build())
        .text_prompt(config.summary_text_prompt.clone())
        .build();

    let (mut indexer_ctx, mut indexer_actor) = Actor::spawn(
        engine.clone(),
        indexer_id.clone(),
        indexer,
        SpawnOptions::builder().exists(SpawnExistsOptions::Restore).build(),
    )
    .await?;

    let indexer_handle = tokio::spawn(async move {
        if let Err(e) = indexer_actor.start(&mut indexer_ctx).await {
            error!("Indexer actor error: {}", e);
        }
    });
    actor_handles.push(indexer_handle);

    // Retriever setup
    let retriever_id = ActorId::of::<Retriever>("/rag/retriever");
    let (mut retriever_ctx, mut retriever_actor) = Actor::spawn(
        engine.clone(),
        retriever_id.clone(),
        Retriever::default(),
        SpawnOptions::builder().exists(SpawnExistsOptions::Reset).build(),
    )
    .await?;

    let retriever_handle = tokio::spawn(async move {
        if let Err(e) = retriever_actor.start(&mut retriever_ctx).await {
            error!("Retriever actor error: {}", e);
        }
    });
    actor_handles.push(retriever_handle);

    let embeddings_id = ActorId::of::<Embeddings>("/rag/embeddings");
    let (mut embeddings_ctx, mut embeddings_actor) = Actor::spawn(
        engine.clone(),
        embeddings_id.clone(),
        Embeddings::default(),
        SpawnOptions::builder().exists(SpawnExistsOptions::Reset).build(),
    )
    .await?;

    let embeddings_handle = tokio::spawn(async move {
        if let Err(e) = embeddings_actor.start(&mut embeddings_ctx).await {
            error!("Embeddings actor error: {}", e);
        }
    });
    actor_handles.push(embeddings_handle);

    let rerank_id = ActorId::of::<Rerank>("/rag/rerank");
    let (mut rerank_ctx, mut rerank_actor) = Actor::spawn(
        engine.clone(),
        rerank_id.clone(),
        Rerank::default(),
        SpawnOptions::builder().exists(SpawnExistsOptions::Reset).build(),
    )
    .await?;

    let rerank_handle = tokio::spawn(async move {
        if let Err(e) = rerank_actor.start(&mut rerank_ctx).await {
            error!("Rerank actor error: {}", e);
        }
    });
    actor_handles.push(rerank_handle);

    let chat_id = ActorId::of::<Chat>("/rag/chat");
    let (mut chat_ctx, mut chat_actor) = Actor::spawn(
        engine.clone(),
        chat_id.clone(),
        Chat::builder()
            .model(config.chat_model.clone())
            .endpoint(config.chat_endpoint.clone())
            .messages_number_limit(config.chat_messages_limit)
            .generation_options(GenerationOptions::default().num_ctx(config.chat_context_length))
            .build(),
        SpawnOptions::builder().exists(SpawnExistsOptions::Reset).build(),
    )
    .await?;

    let chat_handle = tokio::spawn(async move {
        if let Err(e) = chat_actor.start(&mut chat_ctx).await {
            error!("Chat actor error: {}", e);
        }
    });
    actor_handles.push(chat_handle);

    // Think Chat setup
    let think_chat_id = ActorId::of::<Chat>("/rag/think_chat");
    let (mut think_chat_ctx, mut think_chat_actor) = Actor::spawn(
        engine.clone(),
        think_chat_id.clone(),
        Chat::builder()
            .model(config.think_model.clone())
            .endpoint(config.chat_endpoint.clone())
            .messages_number_limit(config.think_messages_limit)
            .generation_options(GenerationOptions::default().num_ctx(config.think_context_length))
            .build(),
        SpawnOptions::builder().exists(SpawnExistsOptions::Reset).build(),
    )
    .await?;

    let think_chat_handle = tokio::spawn(async move {
        if let Err(e) = think_chat_actor.start(&mut think_chat_ctx).await {
            error!("Think Chat actor error: {}", e);
        }
    });
    actor_handles.push(think_chat_handle);

    // Create app state
    let data = web::Data::new(AppState {
        config: config.clone(),
        engine: engine.clone(),
        indexer: indexer_id,
        retriever: retriever_id,
        embeddings: embeddings_id,
        rerank: rerank_id,
        chat: chat_id,
        think_chat: think_chat_id,
    });

    // Create and run server
    let rag_endpoint_host = config.rag_endpoint.host_str().unwrap_or("0.0.0.0");
    let rag_endpoint_port = config.rag_endpoint.port().unwrap_or(5766);

    let server = HttpServer::new(move || {
        let cors = Cors::default().allow_any_origin().allow_any_method().allow_any_header().max_age(3600);

        App::new()
            .wrap(Logger::default().exclude_regex("^/health"))
            .wrap(cors)
            .app_data(data.clone())
            .app_data(
                actix_multipart::form::MultipartFormConfig::default()
                    .memory_limit(UPLOAD_MEMORY_LIMIT)
                    .total_limit(UPLOAD_TOTAL_LIMIT),
            )
            .service(Files::new("/templates", "tools/cognition/templates"))
            // Add the dynamic swagger-initializer.js route before the static files
            .route("/docs/swagger-ui/dist/swagger-initializer.js", web::get().to(swagger_initializer))
            // Then serve the rest of the static files
            .service(Files::new("/docs", "tools/cognition/docs").prefer_utf8(true).use_last_modified(true))
            // Serve dashboard at root
            .route("/", web::get().to(dashboard))
            .route("/health", web::get().to(health))
            .route("/hello", web::get().to(hello))
            .route("/reset", web::post().to(reset))
            .route("/index", web::post().to(index))
            .route("/retrieve", web::post().to(retrieve))
            .route("/ask", web::post().to(ask))
            .route("/chat", web::post().to(chat))
            .route("/think", web::post().to(think))
            .route("/upload", web::post().to(upload))
            .route("/upload", web::route().method(Method::OPTIONS).to(upload_config))
            .route("/delete_source", web::post().to(delete_source))
            .route("/embed", web::post().to(embed))
            .route("/rerank", web::post().to(rerank))
            .route("/sources", web::get().to(list_sources))
            .route(
                "/api-docs/openapi.json",
                web::get().to(|data: web::Data<AppState>| async move {
                    let mut openapi = ApiDoc::openapi();
                    openapi.servers.get_or_insert_with(Vec::new).push(
                        ServerBuilder::new()
                            .url(data.config.rag_endpoint.as_str())
                            .description(Some("Rag Endpoint"))
                            .build(),
                    );
                    HttpResponse::Ok().json(openapi)
                }),
            )
            // Compatibility
            .route("/api/chat", web::post().to(chat))
            .route("/api/embed", web::post().to(embed))
    })
    .bind((rag_endpoint_host, rag_endpoint_port))?
    .run();

    // Run the server
    match server.await {
        Ok(_) => info!("Server stopped"),
        Err(e) => error!("Server error: {}", e),
    }

    // Clean up
    for handle in actor_handles {
        handle.abort();
    }

    Ok(())
}
