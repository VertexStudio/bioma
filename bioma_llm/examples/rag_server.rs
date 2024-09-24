use actix_cors::Cors;
use actix_web::{web, App, HttpResponse, HttpServer, Responder};
use bioma_actor::prelude::*;
use bioma_llm::prelude::*;
use tokio::sync::Mutex;
use tracing::{error, info, warn};

/// Example of a RAG server using the Bioma Actor framework
///
/// CURL examples:
///
/// Reset the engine:
/// curl -X POST http://localhost:8080/reset
///
/// Index some files:
/// curl -X POST http://localhost:8080/index -H "Content-Type: application/json" -d '{"globs": ["/Users/rozgo/BiomaAI/bioma/bioma_*/**/*.rs"], "chunk_capacity": {"start": 500, "end": 2000}, "chunk_overlap": 200}'
///
/// Retrieve context:
/// curl -X POST http://localhost:8080/retrieve -H "Content-Type: application/json" -d '{"query": "Can I make a game with Bioma?", "threshold": 0.0, "limit": 10}'

struct AppState {
    engine: Engine,
    indexer_relay_ctx: Mutex<ActorContext<Relay>>,
    retriever_relay_ctx: Mutex<ActorContext<Relay>>,
    indexer_actor_id: ActorId,
    retriever_actor_id: ActorId,
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
    let engine = Engine::connect("ws://localhost:9123", EngineOptions::default()).await?;

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

    // Create the app state
    let data = web::Data::new(AppState {
        engine: engine.clone(),
        indexer_relay_ctx: Mutex::new(indexer_relay_ctx),
        retriever_relay_ctx: Mutex::new(retriever_relay_ctx),
        indexer_actor_id: indexer_actor_id,
        retriever_actor_id: retriever_actor_id.clone(),
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
    })
    .bind("0.0.0.0:8080")?
    .run()
    .await;

    match server_task {
        Ok(_) => info!("Server stopped"),
        Err(e) => error!("Server error: {}", e),
    }

    retriever_handle.abort();
    indexer_handle.abort();

    Ok(())
}
