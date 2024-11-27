use actix_cors::Cors;
use actix_multipart::form::{json::Json as MpJson, tempfile::TempFile, MultipartForm};
use actix_web::{middleware::Logger, web, App, HttpResponse, HttpServer, Responder};
use bioma_actor::prelude::*;
use bioma_llm::prelude::*;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::sync::Arc;
use tokio::sync::Mutex;
use tracing::{error, info, warn};
use walkdir::WalkDir;
use zip::ZipArchive;

/// Example of a RAG server using the Bioma Actor framework
///
/// CURL examples:
///
/// Reset the engine:
/// curl -X POST http://localhost:8080/reset
///
/// Index some files:
/// curl -X POST http://localhost:8080/index -H "Content-Type: application/json" -d '{"globs": ["/Users/rozgo/BiomaAI/bioma/bioma_*/**/*.rs"]}'
///
/// Retrieve context:
/// curl -X POST http://localhost:8080/retrieve -H "Content-Type: application/json" -d '{"query": "Can I make a game with Bioma?"}'
///
/// Ask a question:
/// curl -X POST http://localhost:8080/ask -H "Content-Type: application/json" -d '{"query": "Can I make a game with Bioma?"}'
///
/// Upload a file:
/// curl -X POST http://localhost:8080/upload -F 'file=@/Users/rozgo/BiomaAI/bioma/README.md' -F 'metadata={"path": "temp0/temp1/README.md"};type=application/json'

struct AppState {
    engine: Engine,
    indexer_actor: Arc<IndexerActor>,
    retriever_actor: Arc<RetrieverActor>,
    embeddings_actor: Arc<EmbeddingsActor>,
    rerank_actor: Arc<RerankActor>,
    chat_relay_ctx: Mutex<ActorContext<Relay>>,
    chat_actor_id: ActorId,
}

struct IndexerActor {
    indexer_ctx: Mutex<ActorContext<Indexer>>,
    indexer_actor: Mutex<Indexer>,
}

struct RetrieverActor {
    retriever_ctx: Mutex<ActorContext<Retriever>>,
    retriever_actor: Mutex<Retriever>,
}

struct EmbeddingsActor {
    embeddings_ctx: Mutex<ActorContext<Embeddings>>,
    embeddings_actor: Mutex<Embeddings>,
}

struct RerankActor {
    rerank_ctx: Mutex<ActorContext<Rerank>>,
    rerank_actor: Mutex<Rerank>,
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
struct Metadata {
    path: std::path::PathBuf,
}

#[derive(Debug, MultipartForm)]
struct Upload {
    #[multipart(limit = "100MB")]
    file: TempFile,
    #[multipart(rename = "metadata")]
    metadata: MpJson<Metadata>,
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

    // If uploading a zip file but target doesn't end in .zip, append original filename
    let temp_file_path = if form.file.file_name.as_ref().map_or(false, |name| name.ends_with(".zip")) {
        if target_dir.extension().map_or(false, |ext| ext == "zip") {
            // If target ends in .zip, use it directly
            output_dir.join(&target_dir)
        } else {
            // If target is a directory, append the original filename
            let original_name =
                form.file.file_name.as_ref().map(|name| name.to_string()).unwrap_or_else(|| "uploaded.zip".to_string());
            output_dir.join(&target_dir).join(original_name)
        }
    } else {
        output_dir.join(&target_dir)
    };

    // Create parent directory if it doesn't exist
    if let Err(e) = tokio::fs::create_dir_all(temp_file_path.parent().unwrap_or(&temp_file_path)).await {
        error!("Failed to create target directory: {}", e);
        return HttpResponse::InternalServerError().json(json!({
            "error": "Failed to create target directory",
            "details": e.to_string()
        }));
    }

    match form.file.file.persist(&temp_file_path) {
        Ok(_) => {
            info!("File uploaded successfully to temporary location");

            let (message, paths) = if temp_file_path.extension().map_or(false, |ext| ext == "zip") {
                let file = match std::fs::File::open(&temp_file_path) {
                    Ok(f) => f,
                    Err(e) => {
                        error!("Error opening zip file: {:?}", e);
                        return HttpResponse::InternalServerError().json(json!({
                            "error": "Failed to open zip file",
                            "details": e.to_string()
                        }));
                    }
                };

                let mut archive = match ZipArchive::new(file) {
                    Ok(a) => a,
                    Err(e) => {
                        error!("Error creating zip archive: {:?}", e);
                        return HttpResponse::InternalServerError().json(json!({
                            "error": "Failed to read zip archive",
                            "details": e.to_string()
                        }));
                    }
                };

                // Extract to the parent directory of the zip file
                let extract_to = temp_file_path.parent().unwrap_or(&temp_file_path);
                if let Err(e) = archive.extract(extract_to) {
                    error!("Error extracting zip archive: {:?}", e);
                    return HttpResponse::InternalServerError().json(json!({
                        "error": "Failed to extract zip archive",
                        "details": e.to_string()
                    }));
                }

                let mut files = Vec::new();
                // Walk through the extracted directory
                for entry in WalkDir::new(extract_to).into_iter().filter_map(|e| e.ok()) {
                    if entry.file_type().is_file() && entry.path() != temp_file_path {
                        files.push(entry.path().to_path_buf());
                    }
                }

                // Delete the temporary zip file after extraction
                if let Err(e) = std::fs::remove_file(&temp_file_path) {
                    warn!("Error removing temporary zip file: {:?}", e);
                }

                (format!("Zip file extracted {} files successfully", files.len()), files)
            } else {
                ("File uploaded successfully".to_string(), vec![temp_file_path])
            };

            HttpResponse::Ok().json(Uploaded { message, paths, size: form.file.size })
        }
        Err(e) => {
            error!("Error saving file: {:?}", e);
            HttpResponse::InternalServerError().json(json!({
                "error": "Failed to save uploaded file",
                "details": e.to_string()
            }))
        }
    }
}

async fn index(body: web::Json<IndexGlobs>, data: web::Data<AppState>) -> HttpResponse {
    let mut indexer_ctx = data.indexer_actor.indexer_ctx.lock().await;
    let mut indexer_actor = data.indexer_actor.indexer_actor.lock().await;

    let index_globs = body.clone();

    info!("Sending message to indexer actor");
    let response = indexer_actor.handle(&mut indexer_ctx, &index_globs).await;
    match response {
        Ok(_) => HttpResponse::Ok().body("OK"),
        Err(e) => HttpResponse::InternalServerError().body(e.to_string()),
    }
}

async fn retrieve(body: web::Json<RetrieveContext>, data: web::Data<AppState>) -> HttpResponse {
    let mut retriever_ctx = data.retriever_actor.retriever_ctx.lock().await;
    let mut retriever_actor = data.retriever_actor.retriever_actor.lock().await;

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
    info!("Received ask query: {:#?}", query);

    // Try to lock the chat_relay_ctx without waiting
    let chat_relay_ctx = match data.chat_relay_ctx.try_lock() {
        Ok(ctx) => ctx,
        Err(_) => {
            error!("Resource busy: could not acquire lock on chat_relay_ctx");
            return HttpResponse::ServiceUnavailable().body("Chat resource busy");
        }
    };

    info!("Sending message to retriever actor");
    let retrieved = {
        let mut retriever_ctx = data.retriever_actor.retriever_ctx.lock().await;
        let mut retriever_actor = data.retriever_actor.retriever_actor.lock().await;
        let retrieve_context = RetrieveContext { query: query.clone(), limit: 5, threshold: 0.0 };
        retriever_actor.handle(&mut retriever_ctx, &retrieve_context).await
    };

    match retrieved {
        Ok(mut context) => {
            // Reverse context to put most important last
            context.context.reverse();

            info!("Context fetched: {:#?}", context);
            let context_content = context.to_markdown();

            // Create chat conversation with all messages
            let mut conversation = body.messages.clone();

            // Insert context to conversation as a system message, at one position before the last message
            let context_message = ChatMessage::system(
                "You are a helpful programming assistant. Format your response in markdown. Use the following context to answer the user's query: \n\n"
                    .to_string()
                    + &context_content,
            );
            if conversation.len() > 0 {
                conversation.insert(conversation.len() - 1, context_message);
            } else {
                conversation.push(context_message);
            }

            info!("Sending context to chat actor");
            let chat_response = chat_relay_ctx
                .send::<Chat, ChatMessages>(
                    ChatMessages { messages: conversation.clone(), restart: false, persist: true },
                    &data.chat_actor_id,
                    SendOptions::builder().timeout(std::time::Duration::from_secs(100)).build(),
                )
                .await;
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
}

async fn ask(body: web::Json<AskQuery>, data: web::Data<AppState>) -> HttpResponse {
    info!("Received ask query: {:#?}", &body.query);

    // Try to lock the chat_relay_ctx without waiting
    let chat_relay_ctx = match data.chat_relay_ctx.try_lock() {
        Ok(ctx) => ctx,
        Err(_) => {
            error!("Resource busy: could not acquire lock on chat_relay_ctx");
            return HttpResponse::ServiceUnavailable().body("Chat resource busy");
        }
    };

    info!("Sending message to retriever actor");
    let retrieved = {
        let mut retriever_ctx = data.retriever_actor.retriever_ctx.lock().await;
        let mut retriever_actor = data.retriever_actor.retriever_actor.lock().await;
        let retrieve_context = RetrieveContext { query: body.query.clone(), limit: 5, threshold: 0.0 };
        retriever_actor.handle(&mut retriever_ctx, &retrieve_context).await
    };

    match retrieved {
        Ok(context) => {
            info!("Context fetched: {:#?}", context);
            let context_content = context.to_markdown();

            // Create chat conversation
            let mut conversation = vec![];

            // Add context to conversation as a system message
            let context_message = ChatMessage::system(
                "You are a helpful programming assistant. Format your response in markdown. Use the following context to answer the user's query: \n\n"
                    .to_string()
                    + &context_content,
            );
            conversation.push(context_message);

            // Add user's query to conversation
            let user_query = ChatMessage::user(body.query.clone());
            conversation.push(user_query);

            // Sending context and user query to chat actor
            info!("Sending context to chat actor");
            let chat_response = chat_relay_ctx
                .send::<Chat, ChatMessages>(
                    ChatMessages { messages: conversation.clone(), restart: true, persist: false },
                    &data.chat_actor_id,
                    SendOptions::builder().timeout(std::time::Duration::from_secs(100)).build(),
                )
                .await;
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
    let mut indexer_ctx = data.indexer_actor.indexer_ctx.lock().await;
    let mut indexer_actor = data.indexer_actor.indexer_actor.lock().await;

    let delete_source = body.clone();

    info!("Sending delete message to indexer actor for sources: {:?}", body.sources);
    let response = indexer_actor.handle(&mut indexer_ctx, &delete_source).await;

    match response {
        Ok(result) => {
            info!(
                "Deleted {} embeddings. Successfully deleted sources: {:?}. Not found sources: {:?}",
                result.deleted_embeddings, result.deleted_sources, result.not_found_sources
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
    let mut embeddings_actor = data.embeddings_actor.embeddings_actor.lock().await;
    let mut embeddings_ctx = data.embeddings_actor.embeddings_ctx.lock().await;

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
        match embeddings_actor.handle(&mut embeddings_ctx, &GenerateTextEmbeddings { texts: chunk.to_vec() }).await {
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
    let generated_embeddings = GeneratedTextEmbeddings { embeddings: all_embeddings };
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

    let mut rerank_actor = data.rerank_actor.rerank_actor.lock().await;
    let mut rerank_ctx = data.rerank_actor.rerank_ctx.lock().await;

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

    // Spawn the indexer actor, if it already exists, restore it, otherwise reset it
    let indexer_actor_id = ActorId::of::<Indexer>("/indexer");
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
        Arc::new(IndexerActor { indexer_ctx: Mutex::new(indexer_ctx), indexer_actor: Mutex::new(indexer_actor) });

    // Spawn a retriever actor, reset if it already exists, otherwise create it
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

    let retriever_actor = Arc::new(RetrieverActor {
        retriever_ctx: Mutex::new(retriever_ctx),
        retriever_actor: Mutex::new(retriever_actor),
    });

    let chat = Chat::builder().model("llama3.2".into()).build();

    let chat_actor_id = ActorId::of::<Chat>("/chat");
    let (mut chat_ctx, mut chat_actor) = Actor::spawn(
        engine.clone(),
        chat_actor_id.clone(),
        chat,
        SpawnOptions::builder().exists(SpawnExistsOptions::Reset).build(),
    )
    .await
    .unwrap();
    let chat_handle = tokio::spawn(async move {
        if let Err(e) = chat_actor.start(&mut chat_ctx).await {
            error!("Chat actor error: {}", e);
        }
    });

    // Spawn a relay actor to send messages to chat
    let chat_relay_id = ActorId::of::<Relay>("/relay/chat");
    let (chat_relay_ctx, _chat_relay_actor) = Actor::spawn(
        engine.clone(),
        chat_relay_id.clone(),
        Relay,
        SpawnOptions::builder().exists(SpawnExistsOptions::Reset).build(),
    )
    .await
    .unwrap();

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

    let embeddings_actor = Arc::new(EmbeddingsActor {
        embeddings_ctx: Mutex::new(embeddings_ctx),
        embeddings_actor: Mutex::new(embeddings_actor),
    });

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

    let rerank_actor =
        Arc::new(RerankActor { rerank_ctx: Mutex::new(rerank_ctx), rerank_actor: Mutex::new(rerank_actor) });

    // Create the app state
    let data = web::Data::new(AppState {
        engine: engine.clone(),
        indexer_actor,
        retriever_actor,
        embeddings_actor,
        rerank_actor,
        chat_relay_ctx: Mutex::new(chat_relay_ctx),
        chat_actor_id: chat_actor_id.clone(),
    });

    // Create the server
    let server_task = HttpServer::new(move || {
        let cors = Cors::default().allow_any_origin().allow_any_method().allow_any_header().max_age(3600);

        App::new()
            .wrap(Logger::default())
            .wrap(cors)
            .app_data(data.clone())
            .route("/health", web::get().to(health))
            .route("/hello", web::post().to(hello))
            .route("/reset", web::post().to(reset))
            .route("/index", web::post().to(index))
            .route("/retrieve", web::post().to(retrieve))
            .route("/ask", web::post().to(ask))
            .route("/chat", web::post().to(self::chat))
            .route("/upload", web::post().to(upload))
            .route("/delete_source", web::post().to(delete_source))
            .route("/api/embed", web::post().to(embed))
            .route("/rerank", web::post().to(rerank))
    })
    .bind("0.0.0.0:8080")?
    .run()
    .await;

    match server_task {
        Ok(_) => info!("Server stopped"),
        Err(e) => error!("Server error: {}", e),
    }

    chat_handle.abort();

    Ok(())
}
