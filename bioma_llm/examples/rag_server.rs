use actix_cors::Cors;
use actix_multipart::form::{json::Json as MpJson, tempfile::TempFile, MultipartForm};
use actix_web::{middleware::Logger, web, App, HttpResponse, HttpServer, Responder};
use base64::Engine as Base64Engine;
use bioma_actor::prelude::*;
use bioma_llm::prelude::*;
use embeddings::EmbeddingContent;
use futures_util::StreamExt;
use indexer::Metadata;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::error::Error as StdError;
use tracing::{error, info};

/// Example of a RAG server using the Bioma Actor framework
///
/// CURL (examples)[docs/examples.sh]

const CHAT_DEFAULT_PROMPT: &str = "You are, Bioma, a helpful assistant. Your creator is Vertex Studio, a games and simulation company. Format your response in markdown. Use the following context to answer the user's query: \n\n";

#[derive(Debug, Serialize, Deserialize)]
struct UserActor {}

impl Actor for UserActor {
    type Error = SystemActorError;

    async fn start(&mut self, _ctx: &mut ActorContext<Self>) -> Result<(), Self::Error> {
        Ok(())
    }
}

struct AppState {
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
}

#[derive(Serialize)]
struct ChatResponse {
    #[serde(flatten)]
    response: ChatMessageResponse,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    context: Vec<ChatMessage>,
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
        let user_messages: Vec<_> = body
            .messages
            .iter()
            .filter(|message| message.role == ollama_rs::generation::chat::MessageRole::User)
            .collect();

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
        Ok(context) => {
            // Create a system message containing the retrieved context
            let context_content = context.to_markdown();
            let context_message = ChatMessage::system(format!("{}{}", CHAT_DEFAULT_PROMPT, context_content));

            // Build the conversation by inserting the context message before the last user message
            let conversation = {
                let mut conv = Vec::with_capacity(body.messages.len() + 1);
                if !body.messages.is_empty() {
                    conv.extend_from_slice(&body.messages[..body.messages.len() - 1]);
                    conv.push(context_message);
                    conv.push(body.messages[body.messages.len() - 1].clone());
                } else {
                    conv.push(context_message);
                }
                conv
            };

            // Set up streaming channel with sufficient buffer for chat responses
            let (tx, rx) = tokio::sync::mpsc::channel(1000);

            // Spawn a task to handle the chat stream processing
            tokio::spawn(async move {
                let chat_request =
                    ChatMessages { messages: conversation.clone(), restart: false, persist: true, stream: true };

                match user_actor
                    .send::<Chat, ChatMessages>(
                        chat_request,
                        &data.chat,
                        SendOptions::builder().timeout(std::time::Duration::from_secs(60)).build(),
                    )
                    .await
                {
                    Ok(mut stream) => {
                        while let Some(response) = stream.next().await {
                            match response {
                                Ok(chunk) => {
                                    // Include full conversation context only in the final message
                                    let response = ChatResponse {
                                        response: chunk.clone(),
                                        context: if chunk.done { conversation.clone() } else { vec![] },
                                    };

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

            // Stream responses as Server-Sent Events (SSE)
            HttpResponse::Ok().content_type("text/event-stream").streaming::<_, Box<dyn StdError>>(
                tokio_stream::wrappers::ReceiverStream::new(rx).map(|result| match result {
                    Ok(response) => {
                        let mut data = String::with_capacity(256);
                        data.push_str("data: ");
                        data.push_str(&serde_json::to_string(&response).unwrap_or_default());
                        data.push_str("\n\n");
                        Ok(web::Bytes::from(data))
                    }
                    Err(e) => {
                        let mut error = String::with_capacity(e.len() + 8);
                        error.push_str("error: ");
                        error.push_str(&e);
                        error.push_str("\n\n");
                        Ok(web::Bytes::from(error))
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
}

#[derive(Serialize)]
struct AskResponse {
    #[serde(flatten)]
    response: ChatMessageResponse,
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
        .filter(|message| message.role == ollama_rs::generation::chat::MessageRole::User)
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

            let mut context_message = ChatMessage::system(format!("{}{}", CHAT_DEFAULT_PROMPT, context_content));

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
                                        context_message.images =
                                            Some(vec![ollama_rs::generation::images::Image::from_base64(&base64_data)]);
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
                    ChatMessages { messages: conversation.clone(), restart: false, persist: false, stream: false },
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

    if body.model != "nomic-embed-text" {
        return HttpResponse::BadRequest().body("Invalid model");
    }

    // Check if input is a string or a list of strings
    let texts = match body.input.as_str() {
        Some(s) => vec![s.to_string()],
        None => body.input.as_array().unwrap().iter().map(|s| s.to_string()).collect(),
    };

    let max_text_len = texts.iter().map(|text| text.len()).max().unwrap_or(0);
    info!("Received embed query with {} texts (max. {} chars)", texts.len(), max_text_len);

    const CHUNK_SIZE: usize = 10;
    let mut all_embeddings = Vec::new();

    // Process texts in chunks
    for chunk in texts.chunks(CHUNK_SIZE) {
        match user_actor
            .send_and_wait_reply::<Embeddings, GenerateEmbeddings>(
                GenerateEmbeddings { content: EmbeddingContent::Text(chunk.to_vec()) },
                &data.embeddings,
                SendOptions::builder().timeout(std::time::Duration::from_secs(30)).build(),
            )
            .await
        {
            Ok(chunk_response) => {
                all_embeddings.extend(chunk_response.embeddings);
            }
            Err(e) => {
                error!("Error processing chunk: {:?}", e);
                return HttpResponse::InternalServerError().body(e.to_string());
            }
        }
    }

    // Return combined results
    let generated_embeddings = GeneratedEmbeddings { embeddings: all_embeddings };
    HttpResponse::Ok().json(generated_embeddings)
}

#[derive(Deserialize)]
struct RerankQuery {
    query: String,
    texts: Vec<String>,
}

async fn rerank(body: web::Json<RerankQuery>, data: web::Data<AppState>) -> HttpResponse {
    let user_actor = match data.user_actor().await {
        Ok(actor) => actor,
        Err(e) => return HttpResponse::InternalServerError().body(e.to_string()),
    };

    let max_text_len = body.texts.iter().map(|text| text.len()).max().unwrap_or(0);
    info!("Received rerank query with {} texts (max. {} chars)", body.texts.len(), max_text_len);

    let rank_texts = RankTexts { query: body.query.clone(), texts: body.texts.clone() };

    match user_actor
        .send_and_wait_reply::<Rerank, RankTexts>(
            rank_texts,
            &data.rerank,
            SendOptions::builder().timeout(std::time::Duration::from_secs(30)).build(),
        )
        .await
    {
        Ok(ranked_texts) => HttpResponse::Ok().json(ranked_texts),
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

    // Initialize engine
    let engine = Engine::connect(EngineOptions::builder().endpoint("ws://localhost:9123".into()).build()).await?;

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
        Chat::default(),
        SpawnOptions::builder().exists(SpawnExistsOptions::Reset).build(),
    )
    .await?;

    let chat_handle = tokio::spawn(async move {
        if let Err(e) = chat_actor.start(&mut chat_ctx).await {
            error!("Chat actor error: {}", e);
        }
    });
    actor_handles.push(chat_handle);

    // Create app state
    let data = web::Data::new(AppState {
        engine: engine.clone(),
        indexer: indexer_id,
        retriever: retriever_id,
        embeddings: embeddings_id,
        rerank: rerank_id,
        chat: chat_id,
    });

    // Create and run server
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
            .route("/ask", web::post().to(self::ask))
            .route("/api/chat", web::post().to(self::chat))
            .route("/upload", web::post().to(upload))
            .route("/delete_source", web::post().to(delete_source))
            .route("/api/embed", web::post().to(embed))
            .route("/rerank", web::post().to(rerank))
    })
    .bind("0.0.0.0:5766")?
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
