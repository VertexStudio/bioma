use actix_cors::Cors;
use actix_multipart::form::{json::Json as MpJson, tempfile::TempFile, MultipartForm};
use actix_web::{middleware::Logger, web, App, HttpResponse, HttpServer, Responder};
use base64::Engine as Base64Engine;
use bioma_actor::prelude::*;
use bioma_llm::prelude::*;
use embeddings::EmbeddingContent;
use indexer::Metadata;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::sync::Arc;
use tokio::sync::Mutex;
use tracing::{error, info, warn};

/// Example of a RAG server using the Bioma Actor framework
///
/// CURL (examples)[docs/examples.sh]

struct ActorHandle<T: Actor> {
    ctx: Mutex<ActorContext<T>>,
    actor: Mutex<T>,
}

struct AppState {
    engine: Engine,
    indexer_actor: Arc<ActorHandle<Indexer>>,
    retriever_actor: Arc<ActorHandle<Retriever>>,
    embeddings_actor: Arc<ActorHandle<Embeddings>>,
    rerank_actor: Arc<ActorHandle<Rerank>>,
    chat_actor: Arc<ActorHandle<Chat>>,
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
    let mut indexer_ctx = data.indexer_actor.ctx.lock().await;
    let mut indexer_actor = data.indexer_actor.actor.lock().await;

    let index_globs = body.clone();

    info!("Sending message to indexer actor");
    let response = indexer_actor.handle(&mut indexer_ctx, &index_globs).await;
    match response {
        Ok(_) => HttpResponse::Ok().body("OK"),
        Err(e) => HttpResponse::InternalServerError().body(e.to_string()),
    }
}

async fn retrieve(body: web::Json<RetrieveContext>, data: web::Data<AppState>) -> HttpResponse {
    let mut retriever_ctx = data.retriever_actor.ctx.lock().await;
    let mut retriever_actor = data.retriever_actor.actor.lock().await;

    let retrieve_context = body.clone();

    info!("Sending message to retriever actor");
    let response = retriever_actor.handle(&mut retriever_ctx, &retrieve_context).await;

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

async fn chat(body: web::Json<ChatQuery>, data: web::Data<AppState>) -> HttpResponse {
    // Build query from all user messages
    let query = body
        .messages
        .iter()
        .filter(|message| message.role == ollama_rs::generation::chat::MessageRole::User)
        .map(|message| message.content.clone())
        .collect::<Vec<String>>()
        .join("\n");
    info!("Received chat query: {:#?}", query);

    info!("Sending message to retriever actor");
    let retrieved = {
        let mut retriever_ctx = data.retriever_actor.ctx.lock().await;
        let mut retriever_actor = data.retriever_actor.actor.lock().await;
        let retrieve_context = RetrieveContext {
            query: RetrieveQuery::Text(query.clone()),
            limit: 5,
            threshold: 0.0,
            source: body.source.clone(),
        };
        retriever_actor.handle(&mut retriever_ctx, &retrieve_context).await
    };

    match retrieved {
        Ok(mut context) => {
            context.context.reverse();
            info!("Context fetched: {:#?}", context);
            let context_content = context.to_markdown();

            // Create system message with text context
            let mut context_message = ChatMessage::system(
                "You are a helpful programming assistant. Format your response in markdown. Use the following context to answer the user's query: \n\n"
                    .to_string()
                    + &context_content,
            );

            // If there's an image in the context, use only the first one
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
            if conversation.len() > 0 {
                conversation.insert(conversation.len() - 1, context_message);
            } else {
                conversation.push(context_message);
            }

            info!("Sending context to chat actor");
            let chat_response = {
                let mut chat_ctx = data.chat_actor.ctx.lock().await;
                let mut chat_actor = data.chat_actor.actor.lock().await;
                chat_actor
                    .handle(
                        &mut chat_ctx,
                        &ChatMessages { messages: conversation.clone(), restart: false, persist: true },
                    )
                    .await
            };
            match chat_response {
                Ok(response) => {
                    info!("Chat response: {:#?}", response);
                    HttpResponse::Ok().json(response)
                }
                Err(e) => {
                    error!("Error fetching chat response: {:?}", e);
                    HttpResponse::InternalServerError().body(format!("Error fetching chat response: {}", e))
                }
            }
        }
        Err(e) => {
            error!("Error fetching context: {:?}", e);
            HttpResponse::InternalServerError().body(format!("Error fetching context: {}", e))
        }
    }
}

#[derive(Deserialize)]
struct AskQuery {
    query: String,
    source: Option<String>,
}

async fn ask(body: web::Json<AskQuery>, data: web::Data<AppState>) -> HttpResponse {
    info!("Received ask query: {:#?}", &body.query);

    info!("Sending message to retriever actor");
    let retrieved = {
        let mut retriever_ctx = data.retriever_actor.ctx.lock().await;
        let mut retriever_actor = data.retriever_actor.actor.lock().await;
        let retrieve_context = RetrieveContext {
            query: RetrieveQuery::Text(body.query.clone()),
            limit: 5,
            threshold: 0.0,
            source: body.source.clone(),
        };
        retriever_actor.handle(&mut retriever_ctx, &retrieve_context).await
    };

    match retrieved {
        Ok(context) => {
            info!("Context fetched: {:#?}", context);
            let context_content = context.to_markdown();

            // Create chat conversation
            let mut conversation = vec![];

            // Add context to conversation as a system message
            let mut context_message = ChatMessage::system(
                "You are a helpful programming assistant. Format your response in markdown. Use the following context to answer the user's query: \n\n"
                    .to_string()
                    + &context_content,
            );

            // Add images to the system message if available
            let mut images = Vec::new();
            for ctx in context.context {
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
                                        images.push(ollama_rs::generation::images::Image::from_base64(&base64_data));
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
            if !images.is_empty() {
                context_message.images = Some(images);
            }

            conversation.push(context_message);

            // Add user's query to conversation
            let user_query = ChatMessage::user(body.query.clone());
            conversation.push(user_query);

            // Sending context and user query to chat actor
            info!("Sending context to chat actor");
            let chat_response = {
                let mut chat_ctx = data.chat_actor.ctx.lock().await;
                let mut chat_actor = data.chat_actor.actor.lock().await;
                chat_actor
                    .handle(
                        &mut chat_ctx,
                        &ChatMessages { messages: conversation.clone(), restart: true, persist: false },
                    )
                    .await
            };
            match chat_response {
                Ok(response) => {
                    info!("Chat response for query: {:#?} is: \n{:#?}", body.query, response);
                    HttpResponse::Ok().json(json!({
                        "response": response,
                    }))
                }
                Err(e) => {
                    error!("Error fetching chat response: {:?}", e);
                    HttpResponse::InternalServerError().body(format!("Error fetching chat response: {}", e))
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
    let mut indexer_ctx = data.indexer_actor.ctx.lock().await;
    let mut indexer_actor = data.indexer_actor.actor.lock().await;

    let delete_source = body.clone();

    info!("Sending delete message to indexer actor for sources: {:?}", delete_source);
    let response = indexer_actor.handle(&mut indexer_ctx, &delete_source).await;

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
    let mut embeddings_actor = data.embeddings_actor.actor.lock().await;
    let mut embeddings_ctx = data.embeddings_actor.ctx.lock().await;

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
        match embeddings_actor
            .handle(&mut embeddings_ctx, &GenerateEmbeddings { content: EmbeddingContent::Text(chunk.to_vec()) })
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
    let max_text_len = body.texts.iter().map(|text| text.len()).max().unwrap_or(0);
    info!("Received rerank query with {} texts (max. {} chars)", body.texts.len(), max_text_len);

    let mut rerank_actor = data.rerank_actor.actor.lock().await;
    let mut rerank_ctx = data.rerank_actor.ctx.lock().await;

    println!("Rerank query: {:#?}", body.query);
    println!("Rerank texts: {:#?}", body.texts);

    let response =
        rerank_actor.handle(&mut rerank_ctx, &RankTexts { query: body.query.clone(), texts: body.texts.clone() }).await;

    println!("Rerank response: {:#?}", response);

    match response {
        Ok(response) => HttpResponse::Ok().json(response.texts),
        Err(e) => HttpResponse::InternalServerError().body(e.to_string()),
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Initialize tracing
    let filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info"));
    tracing_subscriber::fmt().with_env_filter(filter).init();

    // Install color backtrace
    color_backtrace::install();
    color_backtrace::BacktracePrinter::new().message("BOOM! 💥").install(color_backtrace::default_output_stream());

    // Initialize the actor system
    let engine_options = EngineOptions::builder().endpoint("ws://localhost:9123".into()).build();
    let engine = Engine::connect(engine_options).await?;

    // Indexer
    let indexer_actor_id = ActorId::of::<Indexer>("/rag/indexer");
    let (mut indexer_ctx, mut indexer_actor) = match Actor::spawn(
        engine.clone(),
        indexer_actor_id.clone(),
        Indexer::default(),
        SpawnOptions::builder().exists(SpawnExistsOptions::Restore).build(),
    )
    .await
    {
        Ok(result) => {
            info!("Indexer actor restored");
            result
        }
        Err(err) => {
            warn!("Error restoring indexer actor: {}, creating new", err);
            Actor::spawn(
                engine.clone(),
                indexer_actor_id.clone(),
                Indexer::default(),
                SpawnOptions::builder().exists(SpawnExistsOptions::Reset).build(),
            )
            .await
            .unwrap()
        }
    };

    indexer_actor.init(&mut indexer_ctx).await.unwrap();

    let indexer_actor =
        Arc::new(ActorHandle::<Indexer> { ctx: Mutex::new(indexer_ctx), actor: Mutex::new(indexer_actor) });

    // Retriever
    let retriever_actor_id = ActorId::of::<Retriever>("/rag/retriever");
    let (mut retriever_ctx, mut retriever_actor) = Actor::spawn(
        engine.clone(),
        retriever_actor_id.clone(),
        Retriever::default(),
        SpawnOptions::builder().exists(SpawnExistsOptions::Reset).build(),
    )
    .await
    .unwrap();

    retriever_actor.init(&mut retriever_ctx).await.unwrap();

    let retriever_actor =
        Arc::new(ActorHandle::<Retriever> { ctx: Mutex::new(retriever_ctx), actor: Mutex::new(retriever_actor) });

    let chat = Chat::builder().model("llama3.2-vision".into()).build();

    let chat_actor_id = ActorId::of::<Chat>("/rag/chat");
    let (mut chat_ctx, mut chat_actor) = Actor::spawn(
        engine.clone(),
        chat_actor_id.clone(),
        chat,
        SpawnOptions::builder().exists(SpawnExistsOptions::Reset).build(),
    )
    .await
    .unwrap();

    chat_actor.init(&mut chat_ctx).await.unwrap();

    let chat_actor = Arc::new(ActorHandle::<Chat> { ctx: Mutex::new(chat_ctx), actor: Mutex::new(chat_actor) });

    // Embeddings
    let embeddings_actor_id = ActorId::of::<Embeddings>("/rag/embeddings");
    let (mut embeddings_ctx, mut embeddings_actor) = Actor::spawn(
        engine.clone(),
        embeddings_actor_id.clone(),
        Embeddings::default(),
        SpawnOptions::builder().exists(SpawnExistsOptions::Reset).build(),
    )
    .await
    .unwrap();

    embeddings_actor.init(&mut embeddings_ctx).await.unwrap();

    let embeddings_actor =
        Arc::new(ActorHandle::<Embeddings> { ctx: Mutex::new(embeddings_ctx), actor: Mutex::new(embeddings_actor) });

    // Rerank
    let rerank_actor_id = ActorId::of::<Rerank>("/rag/rerank");
    let (mut rerank_ctx, mut rerank_actor) = Actor::spawn(
        engine.clone(),
        rerank_actor_id.clone(),
        Rerank::default(),
        SpawnOptions::builder().exists(SpawnExistsOptions::Reset).build(),
    )
    .await
    .unwrap();

    rerank_actor.init(&mut rerank_ctx).await.unwrap();

    let rerank_actor = Arc::new(ActorHandle::<Rerank> { ctx: Mutex::new(rerank_ctx), actor: Mutex::new(rerank_actor) });

    // Create the app state
    let data = web::Data::new(AppState {
        engine: engine.clone(),
        indexer_actor,
        retriever_actor,
        embeddings_actor,
        rerank_actor,
        chat_actor,
    });

    // Create the server
    let server_task = HttpServer::new(move || {
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
    .run()
    .await;

    match server_task {
        Ok(_) => info!("Server stopped"),
        Err(e) => error!("Server error: {}", e),
    }

    Ok(())
}
