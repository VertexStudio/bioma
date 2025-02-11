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
    AskQueryRequestSchema, ChatQueryRequestSchema, DeleteSourceRequestSchema, EmbeddingsQueryRequestSchema,
    IndexGlobsRequestSchema, RankTextsRequestSchema, RetrieveContextRequest, RetrieveOutputFormat,
    ThinkQueryRequestSchema, UploadRequestSchema,
};
use base64::Engine as Base64Engine;
use bioma_actor::prelude::*;
use bioma_llm::{markitdown::MarkitDown, pdf_analyzer::PdfAnalyzer};
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
use std::{collections::HashMap, error::Error as StdError};
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

#[utoipa::path(
    get,
    path = "/health",
    description = "Check health of the server and its services.",
    responses(
        (status = 200, description = "Ok"),
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
        (status = 200, description = "Ok"),
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
        (status = 200, description = "Ok"),
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

#[derive(Debug, Serialize)]
struct Uploaded {
    message: String,
    paths: Vec<std::path::PathBuf>,
    size: usize,
}

impl Default for UploadConfig {
    fn default() -> Self {
        Self { max_file_size: UPLOAD_TOTAL_LIMIT, max_memory_buffer: UPLOAD_MEMORY_LIMIT }
    }
}

#[utoipa::path(
    post,
    path = "/upload",
    description = "Upload files to the server.",
    request_body(content = UploadRequestSchema, content_type = "multipart/form-data"),
    responses(
        (status = 200, description = "Ok"),
    )
)]
async fn upload(MultipartForm(form): MultipartForm<UploadRequestSchema>, data: web::Data<AppState>) -> impl Responder {
    let output_dir = data.engine.local_store_dir().clone();
    let target_dir = form.metadata.path.clone();

    // Determine the target path for the file
    let temp_file_path = if form.file.file_name.as_ref().map_or(false, |name| name.ends_with(".zip")) {
        if target_dir.extension().map_or(false, |ext| ext == "zip") {
            output_dir.join(&target_dir)
        } else {
            let original_name =
                form.file.file_name.as_ref().map(|name| name.to_string()).unwrap_or_else(|| "uploaded.zip".to_string());
            output_dir.join(&target_dir).join(original_name)
        }
    } else {
        output_dir.join(&target_dir)
    };

    // Create parent directory
    if let Some(parent) = temp_file_path.parent() {
        if let Err(e) = tokio::fs::create_dir_all(parent).await {
            error!("Failed to create target directory: {}", e);
            return HttpResponse::InternalServerError().json(json!({
                "error": "Failed to create target directory",
                "details": e.to_string()
            }));
        }
    }

    // Create a temporary file for the upload
    let temp_path = std::env::temp_dir().join(format!("upload_{}", uuid::Uuid::new_v4()));

    // Copy the uploaded content to the temporary file using tokio
    if let Err(e) = tokio::fs::copy(&form.file.file.path(), &temp_path).await {
        error!("Failed to copy uploaded file: {}", e);
        return HttpResponse::InternalServerError().json(json!({
            "error": "Failed to copy uploaded file",
            "details": e.to_string()
        }));
    }

    let temp_path_clone = temp_path.clone();
    let temp_file_path_clone = temp_file_path.clone();

    if temp_file_path.extension().map_or(false, |ext| ext == "zip") {
        // Handle zip file
        let extract_result = tokio::task::spawn_blocking(
            move || -> Result<Vec<std::path::PathBuf>, Box<dyn std::error::Error + Send + Sync>> {
                let file = std::fs::File::open(&temp_path_clone)?;
                let mut archive = zip::ZipArchive::new(file)?;

                // Extract to the parent directory of the zip file
                let extract_to = temp_file_path_clone.parent().unwrap_or(&temp_file_path_clone);

                // Get list of files that will be extracted
                let mut extracted_files = Vec::new();
                for i in 0..archive.len() {
                    let file = archive.by_index(i)?;
                    if !file.name().ends_with('/') {
                        // Skip directories
                        let outpath = extract_to.join(file.name());
                        extracted_files.push(outpath);
                    }
                }

                // Now perform the extraction
                archive.extract(extract_to)?;

                Ok(extracted_files)
            },
        )
        .await;

        // Clean up temporary file
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
        // Handle regular file
        match tokio::fs::copy(&temp_path, &temp_file_path).await {
            Ok(_) => {
                // Clean up source file after successful copy
                if let Err(e) = tokio::fs::remove_file(&temp_path).await {
                    error!("Failed to clean up temporary file: {}", e);
                }

                HttpResponse::Ok().json(Uploaded {
                    message: "File uploaded successfully".to_string(),
                    paths: vec![temp_file_path],
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
    description = "Receives an array of path of files to index.",
    request_body(content = IndexGlobsRequestSchema, examples(
        ("basic" = (summary = "Basic", value = json!({
            "globs": ["./path/to/files/**/*.rs"], 
            "chunk_capacity": {"start": 500, "end": 2000},
            "chunk_overlap": 200
        })))
    )),
    responses(
        (status = 200, description = "Ok"),
    )
)]
async fn index(body: web::Json<IndexGlobsRequestSchema>, data: web::Data<AppState>) -> HttpResponse {
    let user_actor = match data.user_actor().await {
        Ok(actor) => actor,
        Err(e) => return HttpResponse::InternalServerError().body(e.to_string()),
    };

    let index_globs: IndexGlobs = body.clone().into();

    info!("Sending message to indexer actor");
    let response =
        user_actor.send_and_wait_reply::<Indexer, IndexGlobs>(index_globs, &data.indexer, SendOptions::default()).await;
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
    description = "Retrieve context in .md format.",
    request_body(content = RetrieveContextRequest, examples(
        ("basic" = (summary = "Basic", value = json!({
            "type": "Text",
            "query": "What is Bioma?",
            "threshold": 0.0,
            "limit": 10,
            "sources": ["path/to/source1", "path/to/source2"],
            "format": "markdown"
        })))
    )),
    responses(
        (status = 200, description = "Ok"),
    )
)]
async fn retrieve(body: web::Json<RetrieveContextRequest>, data: web::Data<AppState>) -> HttpResponse {
    let user_actor = match data.user_actor().await {
        Ok(actor) => actor,
        Err(e) => return HttpResponse::InternalServerError().body(e.to_string()),
    };

    let retrieve_context: RetrieveContext = body.clone().into();

    info!("Sending message to retriever actor");
    let response = user_actor
        .send_and_wait_reply::<Retriever, RetrieveContext>(
            retrieve_context,
            &data.retriever,
            SendOptions::builder().timeout(std::time::Duration::from_secs(30)).build(),
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
    description =   "For the usage of this endpoint, and more specifically, for the 'tools' and 'tools_actor' field, only send of the two.<br>
                    If you send 'tools', the definitions that you send, wil be send to the model and will return the tool call, without execute them.<br>
                    If you send 'tools_actors', the endpoint with call the client that contains the tools and actually execute them.",
    request_body(content = ChatQueryRequestSchema, examples(
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
    ))
)]
async fn chat(body: web::Json<ChatQueryRequestSchema>, data: web::Data<AppState>) -> HttpResponse {
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
        limit: 5,
        threshold: 0.0,
        sources: body.sources.clone(),
    };

    let context = user_actor
        .send_and_wait_reply::<Retriever, RetrieveContext>(
            retrieve_context,
            &data.retriever,
            SendOptions::builder().timeout(std::time::Duration::from_secs(30)).build(),
        )
        .await;

    match context {
        Ok(mut context) => {
            // Reverse to keep more relevant context near user query
            context.reverse();
            info!("Context fetched: {:#?}", context);

            // Create a system message containing the retrieved context
            let context_content = context.to_markdown();
            let mut context_message = ChatMessage::system(format!("{}{}", data.config.chat_prompt, context_content));

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
                let conversation = {
                    let has_system_message = body.messages.iter().any(|msg| msg.role == MessageRole::System);

                    let mut conv = Vec::with_capacity(body.messages.len() + 1);
                    if !body.messages.is_empty() {
                        if has_system_message {
                            // Use messages as-is if there's already a system message
                            conv.extend_from_slice(&body.messages);
                        } else {
                            // Insert our system message before the last user message
                            conv.extend_from_slice(&body.messages[..body.messages.len() - 1]);
                            conv.push(context_message.clone());
                            conv.push(body.messages[body.messages.len() - 1].clone());
                        }
                    } else if !has_system_message {
                        // Only add our system message if there isn't one already
                        conv.push(context_message.clone());
                    }
                    conv
                };

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
                            SendOptions::default(),
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
                        .send_and_wait_reply::<ToolsHub, ListTools>(
                            ListTools(None),
                            &actor_id,
                            SendOptions::builder().timeout(std::time::Duration::from_secs(30)).build(),
                        )
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
                    For the usage of this endpoint, and more specifically, for the 'tools' and 'tools_actor' field, only send of the two.<br>
                    If you send 'tools', the definitions that you send, wil be send to the model and will return the tool call, without execute them.<br>
                    If you send 'tools_actors', the endpoint with call the client that contains the tools and return the tools information from all tools_actors.",
    request_body(content = ThinkQueryRequestSchema, examples(
        ("Message only" = (summary = "Basic query", value = json!({
            "messages": [
                {
                    "role": "user",
                    "content": "Why is the sky blue?"
                }
            ]
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
        (status = 200, description = "Ok"),
    )
)]
async fn think(body: web::Json<ThinkQueryRequestSchema>, data: web::Data<AppState>) -> HttpResponse {
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
        limit: 5,
        threshold: 0.0,
        sources: body.sources.clone(),
    };

    let retrieved = match user_actor
        .send_and_wait_reply::<Retriever, RetrieveContext>(
            retrieve_context,
            &data.retriever,
            SendOptions::builder().timeout(std::time::Duration::from_secs(30)).build(),
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

    let system_prompt = if !body.tools.is_empty() {
        // If tools are provided in the request, use those directly
        let tools_str = body
            .tools
            .iter()
            .map(|t| {
                let params = serde_json::to_string_pretty(&t.function.parameters)
                    .unwrap_or_else(|_| "Unable to parse parameters".to_string());
                format!(
                    "- {}: {}\n  Parameters:\n{}",
                    t.function.name,
                    t.function.description,
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
                Err(e) => return HttpResponse::InternalServerError().body(format!("Error fetching tools: {}", e)),
            };
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

    let mut conversation = body.messages.clone();
    let has_system_message = conversation.iter().any(|msg| msg.role == MessageRole::System);

    if !conversation.is_empty() && !has_system_message {
        // Only insert our system message if there isn't one already
        conversation.insert(conversation.len() - 1, context_message);
    } else if conversation.is_empty() && !has_system_message {
        // Only add our system message if there isn't one already
        conversation.push(context_message);
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

        // Use send() to get a stream instead of send_and_wait_reply()
        let mut chat_response = match user_actor
            .send::<Chat, ChatMessages>(
                chat_request,
                &data.think_chat,
                SendOptions::builder().timeout(std::time::Duration::from_secs(30)).build(),
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

#[derive(Serialize)]
struct AskResponse {
    #[serde(flatten)]
    response: ChatMessageResponse,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    context: Vec<ChatMessage>,
}

#[utoipa::path(
    post,
    path = "/ask",
    description = "Generates a chat response. Specific response format can be specified.",
    request_body(content = AskQueryRequestSchema, examples(
        ("Basic" = (summary = "Basic", value = json!({
            "model": "llama3.2",
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
    )),
    responses(
        (status = 200, description = "Ok"),
    )
)]
async fn ask(body: web::Json<AskQueryRequestSchema>, data: web::Data<AppState>) -> HttpResponse {
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
        limit: 5,
        threshold: 0.0,
        sources: body.sources.clone(),
    };

    let retrieved = user_actor
        .send_and_wait_reply::<Retriever, RetrieveContext>(
            retrieve_context,
            &data.retriever,
            SendOptions::builder().timeout(std::time::Duration::from_secs(30)).build(),
        )
        .await;

    match retrieved {
        Ok(context) => {
            info!("Context fetched: {:#?}", context);
            let context_content = context.to_markdown();

            let mut context_message = ChatMessage::system(format!("{}{}", data.config.chat_prompt, context_content));

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

            let mut conversation = body.messages.clone();
            if !conversation.is_empty() {
                conversation.insert(conversation.len() - 1, context_message.clone());
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
                    SendOptions::builder().timeout(std::time::Duration::from_secs(60)).build(),
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
    description = "Delete indexed sources.",
    request_body(content = DeleteSourceRequestSchema, examples(
        ("Basic" = (summary = "Basic", value = json!({"sources": ["path/to/source1", "path/to/source2"]}))),
    )),
    responses(
        (status = 200, description = "Ok"),
    )
)]
async fn delete_source(body: web::Json<DeleteSourceRequestSchema>, data: web::Data<AppState>) -> HttpResponse {
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
            SendOptions::builder().timeout(std::time::Duration::from_secs(30)).build(),
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
    description = "Generate embeddings for text or images.",
    request_body(content = EmbeddingsQueryRequestSchema, examples(
        ("Basic" = (summary = "Text", value = json!({
            "input": "This text will generate embeddings",
            "model": "nomic-embed-text"
        }))),
        ("Image" = (summary = "Image", value = json!({
            "model": "nomic-embed-vision",
            "input": "iVBORw0KGgoAAAANSUhEUgAAAAoAAAAKCAIAAAACUFjqAAABRklEQVR4nAA2Acn+A2ql2+Vv1LF7X3Mw2i9cMEBUs0/l0C6/irfF6wPqowTw0ORE00EZ/He1x+LwZ3nDwaZVNIgn6FI8KQabKikArD0j4g6LU2Mz9DpsAgnYGy6195whWQQ4XIk1a74tA98BtQfyE3oQkaA/uufBkIegK+TH6LMh/O44hIio5wAw4umxtkxZNCIf35A4YNshDwNeeHFnHP0YUSelrm8DMioFvjc7QOcZmEBw/pv+SXEH2G+O0ZdiHDTb6wnhAcRk1rkuJLwy/d7DDKTgqOflV5zk7IBgmz0f8J4o5gA4yb3rYzzUyLRXS0bY40xnoY/rtniWFdlrtSHkR/0A1ClG/qVWNyD1CXVkxE4IW5Tj+8qk1sD42XW6TQpPAO7NhmcDxDz092Q2AR8XYKPa1LPkGberOYArt0gkbQEAAP//4hWZNZ4Pc4kAAAAASUVORK5CYII="
        }))),
    )),
    responses(
        (status = 200, description = "Ok"),
    )
)]
async fn embed(body: web::Json<EmbeddingsQueryRequestSchema>, data: web::Data<AppState>) -> HttpResponse {
    let user_actor = match data.user_actor().await {
        Ok(actor) => actor,
        Err(e) => return HttpResponse::InternalServerError().body(e.to_string()),
    };

    // Validate model
    match body.model.as_str() {
        "nomic-embed-text" | "nomic-embed-vision" => (),
        _ => return HttpResponse::BadRequest().body("Invalid model"),
    }

    // Process input based on model type
    let embedding_content = match body.model.as_str() {
        "nomic-embed-text" => {
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
        "nomic-embed-vision" => {
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
        _ => unreachable!(),
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
                        SendOptions::builder().timeout(std::time::Duration::from_secs(120)).build(),
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
                    SendOptions::builder().timeout(std::time::Duration::from_secs(60)).build(),
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
    description = "Rerank texts based on a query.",
    request_body(content = RankTextsRequestSchema, examples(
        ("Basic" = (summary = "Basic", value = json!({
            "query": "What is Deep Learning?",
            "texts": [
                "Deep Learning is learning under water",
                "Deep learning is a branch of machine learning"
            ],
            "raw_scores": false
        })))
    )),
    responses(
        (status = 200, description = "Ok"),
    )
)]
async fn rerank(body: web::Json<RankTextsRequestSchema>, data: web::Data<AppState>) -> HttpResponse {
    let user_actor = match data.user_actor().await {
        Ok(actor) => actor,
        Err(e) => return HttpResponse::InternalServerError().body(e.to_string()),
    };

    let body: RankTexts = body.clone().into();

    let max_text_len = body.texts.iter().map(|text| text.len()).max().unwrap_or(0);
    info!("Received rerank query with {} texts (max. {} chars)", body.texts.len(), max_text_len);
    debug!("Rerank query: {}", body.query);

    let rank_texts = body.clone();

    match user_actor
        .send_and_wait_reply::<Rerank, RankTexts>(
            rank_texts,
            &data.rerank,
            SendOptions::builder().timeout(std::time::Duration::from_secs(120)).build(),
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
    description = "List all indexed sources with their embedding counts.",
    responses(
        (status = 200, description = "List of sources with their embedding counts"),
    )
)]
async fn list_sources(data: web::Data<AppState>) -> HttpResponse {
    let user_actor = match data.user_actor().await {
        Ok(actor) => actor,
        Err(e) => return HttpResponse::InternalServerError().body(e.to_string()),
    };

    info!("Fetching list of sources");
    let response = user_actor
        .send_and_wait_reply::<Retriever, ListSources>(
            ListSources,
            &data.retriever,
            SendOptions::builder().timeout(std::time::Duration::from_secs(30)).build(),
        )
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
    let (mut indexer_ctx, mut indexer_actor) = Actor::spawn(
        engine.clone(),
        indexer_id.clone(),
        Indexer::default(),
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
            .wrap(Logger::default())
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
