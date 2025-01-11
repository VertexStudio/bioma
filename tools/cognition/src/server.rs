use actix_cors::Cors;
use actix_multipart::form::{json::Json as MpJson, tempfile::TempFile, MultipartForm};
use actix_web::{middleware::Logger, web, App, HttpResponse, HttpServer, Responder};
use base64::Engine as Base64Engine;
use bioma_actor::prelude::*;
use bioma_llm::prelude::*;
use clap::Parser;
use config::{Args, Config};
use embeddings::EmbeddingContent;
use futures_util::StreamExt;
use indexer::Metadata;
use ollama_rs::generation::tools::ToolCall;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::error::Error as StdError;
use std::sync::Arc;
use tokio::sync::Mutex;
use tool::Tools;
use tracing::{debug, error, info};
use user::UserActor;

pub mod config;
pub mod tool;
pub mod user;

struct AppState {
    config: Config,
    tools: Arc<Mutex<Tools>>,
    engine: Engine,
    indexer: ActorId,
    retriever: ActorId,
    embeddings: ActorId,
    rerank: ActorId,
    chat: ActorId,
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

async fn health() -> impl Responder {
    HttpResponse::Ok().body("OK")
}

async fn hello() -> impl Responder {
    HttpResponse::Ok().json("Hello world!")
}

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

#[derive(Debug, Deserialize)]
struct UploadMetadata {
    path: std::path::PathBuf,
}

#[derive(Debug, MultipartForm)]
struct Upload {
    #[multipart(limit = "100MB")]
    file: TempFile,
    #[multipart(rename = "metadata")]
    metadata: MpJson<UploadMetadata>,
}

#[derive(Debug, Serialize)]
struct Uploaded {
    message: String,
    paths: Vec<std::path::PathBuf>,
    size: usize,
}

async fn upload(MultipartForm(form): MultipartForm<Upload>, data: web::Data<AppState>) -> impl Responder {
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
        match tokio::fs::rename(&temp_path, &temp_file_path).await {
            Ok(_) => HttpResponse::Ok().json(Uploaded {
                message: "File uploaded successfully".to_string(),
                paths: vec![temp_file_path],
                size: form.file.size,
            }),
            Err(e) => {
                error!("Error moving file: {:?}", e);
                if let Err(copy_err) = tokio::fs::copy(&temp_path, &temp_file_path).await {
                    error!("Error copying file: {:?}", copy_err);
                    return HttpResponse::InternalServerError().json(json!({
                        "error": "Failed to save uploaded file",
                        "details": copy_err.to_string()
                    }));
                }
                // Clean up source file after successful copy
                let _ = tokio::fs::remove_file(&temp_path).await;

                HttpResponse::Ok().json(Uploaded {
                    message: "File uploaded successfully".to_string(),
                    paths: vec![temp_file_path],
                    size: form.file.size,
                })
            }
        }
    }
}

async fn index(body: web::Json<IndexGlobs>, data: web::Data<AppState>) -> HttpResponse {
    let user_actor = match data.user_actor().await {
        Ok(actor) => actor,
        Err(e) => return HttpResponse::InternalServerError().body(e.to_string()),
    };

    let index_globs = body.clone();

    info!("Sending message to indexer actor");
    let response =
        user_actor.send_and_wait_reply::<Indexer, IndexGlobs>(index_globs, &data.indexer, SendOptions::default()).await;
    match response {
        Ok(_) => HttpResponse::Ok().body("OK"),
        Err(e) => HttpResponse::InternalServerError().body(e.to_string()),
    }
}

async fn retrieve(body: web::Json<RetrieveContext>, data: web::Data<AppState>) -> HttpResponse {
    let user_actor = match data.user_actor().await {
        Ok(actor) => actor,
        Err(e) => return HttpResponse::InternalServerError().body(e.to_string()),
    };

    let retrieve_context = body.clone();

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
            let context_content = context.to_markdown();
            HttpResponse::Ok().json(context_content)
        }
        Err(e) => {
            error!("Error fetching context: {:?}", e);
            HttpResponse::InternalServerError().body(e.to_string())
        }
    }
}

#[derive(Deserialize)]
struct ChatQuery {
    messages: Vec<ChatMessage>,
    source: Option<String>,
    format: Option<chat::Schema>,
}

#[derive(Serialize)]
struct ChatResponse {
    #[serde(flatten)]
    response: ChatMessageResponse,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    context: Vec<ChatMessage>,
}

#[derive(Serialize)]
struct ToolResponse {
    server: String,
    tool: String,
    call: ToolCall,
    response: Value,
}

async fn chat(body: web::Json<ChatQuery>, data: web::Data<AppState>) -> HttpResponse {
    let user_actor = match data.user_actor().await {
        Ok(actor) => actor,
        Err(e) => return HttpResponse::InternalServerError().body(e.to_string()),
    };

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
        source: body.source.clone(),
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

            // Build the conversation by inserting the context message before the last user message
            let mut conversation = {
                let mut conv = Vec::with_capacity(body.messages.len() + 1);
                if !body.messages.is_empty() {
                    conv.extend_from_slice(&body.messages[..body.messages.len() - 1]);
                    conv.push(context_message.clone());
                    conv.push(body.messages[body.messages.len() - 1].clone());
                } else {
                    conv.push(context_message.clone());
                }
                conv
            };

            // Generate tool calls if tools are available
            let tools = data.tools.lock().await.list_tools(&user_actor).await;
            let tool_response = if let Ok(tools) = tools {
                if !tools.is_empty() {
                    let conversation = {
                        let mut conv = Vec::with_capacity(body.messages.len() + 1);
                        if !body.messages.is_empty() {
                            conv.extend_from_slice(&body.messages[..body.messages.len() - 1]);
                            conv.push(body.messages[body.messages.len() - 1].clone());
                        } else {
                            conv.push(context_message.clone());
                        }
                        conv
                    };

                    let tool_request = ChatMessages {
                        messages: conversation.clone(),
                        restart: true,
                        persist: false,
                        stream: false,
                        format: body.format.clone(),
                        tools: Some(tools),
                    };
                    let tool_response = user_actor
                        .send_and_wait_reply::<Chat, ChatMessages>(
                            tool_request,
                            &data.chat,
                            SendOptions::builder().timeout(std::time::Duration::from_secs(60)).build(),
                        )
                        .await;
                    info!("Tool response: {:#?}", tool_response);
                    Some(tool_response)
                } else {
                    None
                }
            } else {
                error!("Error fetching tools: {:?}", tools);
                None
            };

            // Set up streaming channel with sufficient buffer for chat responses
            let (tx, rx) = tokio::sync::mpsc::channel(1000);

            // Spawn a task to handle the chat stream processing
            tokio::spawn(async move {
                // If tool calls are generated, call tools and add them to the conversation
                if let Some(Ok(tool_response)) = tool_response {
                    for tool_call in &tool_response.message.tool_calls {
                        let tools = data.tools.lock().await;
                        let tool = tools.get_tool(&tool_call.function.name);
                        if let Some((_tool_info, tool_client)) = tool {
                            // Execute the tool call and get raw result
                            let execution_result = tool_client.call(&user_actor, tool_call).await;

                            // Convert the execution result to a JSON value
                            let result_json = match execution_result {
                                Ok(execution_output) => {
                                    serde_json::to_value(execution_output.content).unwrap_or_default()
                                }
                                Err(e) => serde_json::json!({
                                    "error": format!("Error calling tool: {:?}", e)
                                }),
                            };

                            // Create the structured tool response
                            let formatted_tool_response = ToolResponse {
                                server: tool_client.server.name.clone(),
                                tool: tool_call.function.name.clone(),
                                call: tool_call.clone(),
                                response: result_json,
                            };

                            // Convert to chat message and create response object
                            let tool_result_message =
                                ChatMessage::tool(serde_json::to_string(&formatted_tool_response).unwrap_or_default());
                            let mut streaming_response = tool_response.clone();
                            streaming_response.message = tool_result_message.clone();
                            let chat_stream_response = ChatResponse { response: streaming_response, context: vec![] };

                            // Send the response through the channel
                            if tx.send(Ok(web::Json(chat_stream_response))).await.is_err() {
                                break;
                            }
                            conversation.push(tool_result_message);
                        }
                    }
                }

                let chat_request = ChatMessages {
                    messages: conversation.clone(),
                    restart: true,
                    persist: false,
                    stream: true,
                    format: body.format.clone(),
                    tools: None,
                };

                match user_actor
                    .send::<Chat, ChatMessages>(
                        chat_request,
                        &data.chat,
                        SendOptions::builder().timeout(std::time::Duration::from_secs(60)).build(),
                    )
                    .await
                {
                    Ok(mut stream) => {
                        let mut is_first_message = true;
                        while let Some(response) = stream.next().await {
                            match response {
                                Ok(chunk) => {
                                    // Include context only in the first message
                                    let response = ChatResponse {
                                        response: chunk.clone(),
                                        context: if is_first_message { conversation.clone() } else { vec![] },
                                    };
                                    is_first_message = false;

                                    if tx.send(Ok::<_, String>(web::Json(response))).await.is_err() {
                                        break;
                                    }
                                }
                                Err(e) => {
                                    let _ = tx.send(Err(e.to_string())).await;
                                    break;
                                }
                            }
                        }
                    }
                    Err(e) => {
                        let _ = tx.send(Err(e.to_string())).await;
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
        Err(e) => {
            error!("Error fetching context: {:?}", e);
            HttpResponse::InternalServerError().body(format!("Error fetching context: {}", e))
        }
    }
}

#[derive(Deserialize)]
struct AskQuery {
    messages: Vec<ChatMessage>,
    source: Option<String>,
    format: Option<chat::Schema>,
}

#[derive(Serialize)]
struct AskResponse {
    #[serde(flatten)]
    response: ChatMessageResponse,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    context: Vec<ChatMessage>,
}

async fn ask(body: web::Json<AskQuery>, data: web::Data<AppState>) -> HttpResponse {
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
    info!("Received chat query: {:#?}", query);

    info!("Sending message to retriever actor");
    let retrieve_context = RetrieveContext {
        query: RetrieveQuery::Text(query.clone()),
        limit: 5,
        threshold: 0.0,
        source: body.source.clone(),
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

async fn delete_source(body: web::Json<DeleteSource>, data: web::Data<AppState>) -> HttpResponse {
    let user_actor = match data.user_actor().await {
        Ok(actor) => actor,
        Err(e) => return HttpResponse::InternalServerError().body(e.to_string()),
    };

    let delete_source = body.clone();

    info!("Sending delete message to indexer actor for sources: {:?}", body.source);
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

#[derive(Deserialize)]
struct EmbeddingsQuery {
    model: String,
    input: serde_json::Value,
}

async fn embed(body: web::Json<EmbeddingsQuery>, data: web::Data<AppState>) -> HttpResponse {
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

async fn rerank(body: web::Json<RankTexts>, data: web::Data<AppState>) -> HttpResponse {
    let user_actor = match data.user_actor().await {
        Ok(actor) => actor,
        Err(e) => return HttpResponse::InternalServerError().body(e.to_string()),
    };

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
        Chat::builder().model(config.chat_model.clone()).endpoint(config.chat_endpoint.clone()).build(),
        SpawnOptions::builder().exists(SpawnExistsOptions::Reset).build(),
    )
    .await?;

    let chat_handle = tokio::spawn(async move {
        if let Err(e) = chat_actor.start(&mut chat_ctx).await {
            error!("Chat actor error: {}", e);
        }
    });
    actor_handles.push(chat_handle);

    // Tools setup
    // let tools_user = UserActor::new(&engine, "/rag/client/tool/".into()).await?;
    let mut tools = Tools::new();
    for tool in &config.tools {
        tools.add_tool(&engine, tool.clone(), "/rag".into()).await?;
    }
    // tools.list_tools(&tools_user).await?;

    // Create app state
    let data = web::Data::new(AppState {
        config: config.clone(),
        tools: Arc::new(Mutex::new(tools)),
        engine: engine.clone(),
        indexer: indexer_id,
        retriever: retriever_id,
        embeddings: embeddings_id,
        rerank: rerank_id,
        chat: chat_id,
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
                    .memory_limit(50 * 1024 * 1024)
                    .total_limit(100 * 1024 * 1024),
            )
            .route("/health", web::get().to(health))
            .route("/hello", web::post().to(hello))
            .route("/reset", web::post().to(reset))
            .route("/index", web::post().to(index))
            .route("/retrieve", web::post().to(retrieve))
            .route("/ask", web::post().to(ask))
            .route("/chat", web::post().to(chat))
            .route("/upload", web::post().to(upload))
            .route("/delete_source", web::post().to(delete_source))
            .route("/embed", web::post().to(embed))
            .route("/rerank", web::post().to(rerank))
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
