use actix_cors::Cors;
use actix_multipart::form::{json::Json as MpJson, tempfile::TempFile, MultipartForm};
use actix_web::{web, App, HttpResponse, HttpServer, Responder};
use bioma_actor::prelude::*;
use bioma_llm::prelude::*;
use serde::{Deserialize, Serialize};
use serde_json::json;
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
    indexer_relay_ctx: Mutex<ActorContext<Relay>>,
    retriever_relay_ctx: Mutex<ActorContext<Relay>>,
    chat_relay_ctx: Mutex<ActorContext<Relay>>,
    indexer_actor_id: ActorId,
    retriever_actor_id: ActorId,
    chat_actor_id: ActorId,
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

    // Get the target directory from the metadata path
    let file_path = output_dir.join(&form.metadata.path);
    let file_dir = file_path.parent().expect("Failed to get file directory");

    // Ensure the file_path directory exists
    match tokio::fs::create_dir_all(file_dir).await {
        Ok(_) => (),
        Err(e) => {
            error!("Failed to create file path directory: {}", e);
            return HttpResponse::InternalServerError().json(json!({
                "error": "Failed to create file path directory",
                "details": e.to_string()
            }));
        }
    }

    match form.file.file.persist(&file_path) {
        Ok(_) => {
            info!("File uploaded successfully: {}", file_path.display());

            let (message, paths) = if file_path.extension().map_or(false, |ext| ext == "zip") {
                let file = match std::fs::File::open(&file_path) {
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
                if let Err(e) = archive.extract(file_dir) {
                    error!("Error extracting zip archive: {:?}", e);
                    return HttpResponse::InternalServerError().json(json!({
                        "error": "Failed to extract zip archive",
                        "details": e.to_string()
                    }));
                }

                // Get the name of the zip file without extension to find its extracted contents
                let zip_stem = file_path.file_stem().and_then(|s| s.to_str()).unwrap_or("extracted");
                let extracted_dir = file_dir.join(zip_stem);

                let mut files = Vec::new();
                // Only walk through the specific extracted directory
                for entry in WalkDir::new(&extracted_dir).into_iter().filter_map(|e| e.ok()) {
                    if entry.file_type().is_file() {
                        files.push(entry.path().to_path_buf());
                    }
                }

                // Delete the zip file after extraction
                if let Err(e) = std::fs::remove_file(&file_path) {
                    error!("Error removing zip file: {:?}", e);
                    return HttpResponse::InternalServerError().json(json!({
                        "error": "Failed to cleanup zip file",
                        "details": e.to_string()
                    }));
                }

                (format!("Zip file extracted {} files successfully", files.len()), files)
            } else {
                ("File uploaded successfully".to_string(), vec![file_path])
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
    let index_globs = body.clone();

    let indexer_actor_id = data.indexer_actor_id.clone();

    // Try to lock the indexer_relay_ctx without waiting
    let relay_ctx = match data.indexer_relay_ctx.try_lock() {
        Ok(ctx) => ctx,
        Err(_) => {
            error!("Resource busy: could not acquire lock on indexer_relay_ctx");
            return HttpResponse::ServiceUnavailable().body("Indexer resource busy");
        }
    };

    info!("Sending message to indexer actor");
    let response = relay_ctx.do_send::<Indexer, IndexGlobs>(index_globs, &indexer_actor_id).await;

    match response {
        Ok(_) => HttpResponse::Ok().body("OK"),
        Err(e) => HttpResponse::InternalServerError().body(e.to_string()),
    }
}

async fn retrieve(body: web::Json<RetrieveContext>, data: web::Data<AppState>) -> HttpResponse {
    let retrieve_context = body.clone();

    let retriever_actor_id = data.retriever_actor_id.clone();

    // Try to lock the retriever_relay_ctx without waiting
    let relay_ctx = match data.retriever_relay_ctx.try_lock() {
        Ok(ctx) => ctx,
        Err(_) => {
            error!("Resource busy: could not acquire lock on retriever_relay_ctx");
            return HttpResponse::ServiceUnavailable().body("Retriever resource busy");
        }
    };

    info!("Sending message to retriever actor");
    let response = relay_ctx
        .send::<Retriever, RetrieveContext>(
            retrieve_context,
            &retriever_actor_id,
            SendOptions::builder().timeout(std::time::Duration::from_secs(100)).build(),
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

    let retrieve_context = RetrieveContext { query: query.clone(), limit: 5, threshold: 0.0 };
    let retriever_actor_id = data.retriever_actor_id.clone();

    // Try to lock the retriever_relay_ctx without waiting
    let retriever_relay_ctx = match data.retriever_relay_ctx.try_lock() {
        Ok(ctx) => ctx,
        Err(_) => {
            error!("Resource busy: could not acquire lock on retriever_relay_ctx");
            return HttpResponse::ServiceUnavailable().body("Retriever resource busy");
        }
    };

    // Try to lock the chat_relay_ctx without waiting
    let chat_relay_ctx = match data.chat_relay_ctx.try_lock() {
        Ok(ctx) => ctx,
        Err(_) => {
            error!("Resource busy: could not acquire lock on chat_relay_ctx");
            return HttpResponse::ServiceUnavailable().body("Chat resource busy");
        }
    };

    info!("Sending message to retriever actor");
    let response = retriever_relay_ctx
        .send::<Retriever, RetrieveContext>(
            retrieve_context,
            &retriever_actor_id,
            SendOptions::builder().timeout(std::time::Duration::from_secs(100)).build(),
        )
        .await;

    match response {
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
    info!("Received ask query: {:#?}", body.query);
    let retrieve_context = RetrieveContext { query: body.query.clone(), limit: 5, threshold: 0.0 };
    let retriever_actor_id = data.retriever_actor_id.clone();

    // Try to lock the retriever_relay_ctx without waiting
    let retriever_relay_ctx = match data.retriever_relay_ctx.try_lock() {
        Ok(ctx) => ctx,
        Err(_) => {
            error!("Resource busy: could not acquire lock on retriever_relay_ctx");
            return HttpResponse::ServiceUnavailable().body("Retriever resource busy");
        }
    };

    // Try to lock the chat_relay_ctx without waiting
    let chat_relay_ctx = match data.chat_relay_ctx.try_lock() {
        Ok(ctx) => ctx,
        Err(_) => {
            error!("Resource busy: could not acquire lock on chat_relay_ctx");
            return HttpResponse::ServiceUnavailable().body("Chat resource busy");
        }
    };

    info!("Sending message to retriever actor");
    let response = retriever_relay_ctx
        .send::<Retriever, RetrieveContext>(
            retrieve_context,
            &retriever_actor_id,
            SendOptions::builder().timeout(std::time::Duration::from_secs(100)).build(),
        )
        .await;

    match response {
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
    let indexer_actor_id = data.indexer_actor_id.clone();

    // Try to lock the indexer_relay_ctx without waiting
    let relay_ctx = match data.indexer_relay_ctx.try_lock() {
        Ok(ctx) => ctx,
        Err(_) => {
            error!("Resource busy: could not acquire lock on indexer_relay_ctx");
            return HttpResponse::ServiceUnavailable().body("Indexer resource busy");
        }
    };

    info!("Sending delete message to indexer actor for source: {}", body.source);
    let response = relay_ctx
        .send::<Indexer, DeleteSource>(
            body.clone(),
            &indexer_actor_id,
            SendOptions::builder().timeout(std::time::Duration::from_secs(30)).build(),
        )
        .await;

    match response {
        Ok(result) => {
            info!("Deleted {} embeddings for source: {}", result.deleted_embeddings, body.source);
            HttpResponse::Ok().json(result)
        }
        Err(e) => {
            error!("Error deleting source: {:?}", e);
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

    // Start the indexer actor
    let indexer_handle = tokio::spawn(async move {
        if let Err(e) = indexer_actor.start(&mut indexer_ctx).await {
            error!("Indexer actor error: {}", e);
        }
    });

    // Spawn a retriever actor, reset if it already exists, otherwise create it
    let retriever_actor_id = ActorId::of::<Retriever>("/retriever");
    let (mut retriever_ctx, mut retriever_actor) = Actor::spawn(
        engine.clone(),
        retriever_actor_id.clone(),
        Retriever::default(),
        SpawnOptions::builder().exists(SpawnExistsOptions::Reset).build(),
    )
    .await
    .unwrap();

    // Start the retriever actor
    let retriever_handle = tokio::spawn(async move {
        if let Err(e) = retriever_actor.start(&mut retriever_ctx).await {
            error!("Retriever actor error: {}", e);
        }
    });

    // Spawn a relay actor to send messages to indexer
    let indexer_relay_id = ActorId::of::<Relay>("/relay/indexer");
    let (indexer_relay_ctx, _indexer_relay_actor) = Actor::spawn(
        engine.clone(),
        indexer_relay_id.clone(),
        Relay,
        SpawnOptions::builder().exists(SpawnExistsOptions::Reset).build(),
    )
    .await
    .unwrap();

    // Spawn a relay actor to send messages to retriever
    let retriever_relay_id = ActorId::of::<Relay>("/relay/retriever");
    let (retriever_relay_ctx, _retriever_relay_actor) = Actor::spawn(
        engine.clone(),
        retriever_relay_id.clone(),
        Relay,
        SpawnOptions::builder().exists(SpawnExistsOptions::Reset).build(),
    )
    .await
    .unwrap();

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

    // Create the app state
    let data = web::Data::new(AppState {
        engine: engine.clone(),
        indexer_relay_ctx: Mutex::new(indexer_relay_ctx),
        retriever_relay_ctx: Mutex::new(retriever_relay_ctx),
        chat_relay_ctx: Mutex::new(chat_relay_ctx),
        indexer_actor_id: indexer_actor_id,
        retriever_actor_id: retriever_actor_id.clone(),
        chat_actor_id: chat_actor_id.clone(),
    });

    // Create the server
    let server_task = HttpServer::new(move || {
        let cors = Cors::default().allow_any_origin().allow_any_method().allow_any_header().max_age(3600);

        App::new()
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
            .route("/delete", web::post().to(delete_source))
    })
    .bind("0.0.0.0:8080")?
    .run()
    .await;

    match server_task {
        Ok(_) => info!("Server stopped"),
        Err(e) => error!("Server error: {}", e),
    }

    chat_handle.abort();
    retriever_handle.abort();
    indexer_handle.abort();

    Ok(())
}
