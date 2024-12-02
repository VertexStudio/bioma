use bioma_llm::indexer::{DeleteSource, IndexGlobs};
use bioma_llm::prelude::*;
use bioma_llm::rerank::RankTexts;
use bioma_llm::retriever::{RetrieveContext, RetrieveQuery};
use clap::{Parser, ValueEnum};
use goose::config::GooseConfiguration;
use goose::prelude::*;
use serde_json::json;

const DEFAULT_CHUNK_CAPACITY: usize = 1024;
const DEFAULT_CHUNK_OVERLAP: usize = 256;
const DEFAULT_CHUNK_BATCH_SIZE: usize = 10;
const DEFAULT_RETRIEVER_LIMIT: usize = 5;
const DEFAULT_RETRIEVER_THRESHOLD: f32 = 0.0;

async fn make_request<T: serde::Serialize>(
    user: &mut GooseUser,
    method: GooseMethod,
    path: &str,
    name: &str,
    payload: Option<T>,
) -> TransactionResult {
    let mut request_builder = user.get_request_builder(&method, path)?;

    if let Some(payload) = payload {
        request_builder = request_builder
            .header("Content-Type", "application/json")
            .body(serde_json::to_string(&payload).unwrap_or_default());
    }

    let goose_request = GooseRequest::builder().set_request_builder(request_builder).name(name).build();

    let mut goose = user.request(goose_request).await?;

    if let Ok(response) = &goose.response {
        if !response.status().is_success() {
            return user.set_failure(
                &format!("{} request failed", name.to_lowercase()),
                &mut goose.request,
                Some(response.headers()),
                None,
            );
        }
    }

    Ok(())
}

pub async fn load_test_health(user: &mut GooseUser) -> TransactionResult {
    make_request::<()>(user, GooseMethod::Get, "/health", "Health Check", None).await
}

pub async fn load_test_hello(user: &mut GooseUser) -> TransactionResult {
    let payload = json!("");

    make_request(user, GooseMethod::Post, "/hello", "Hello", Some(payload)).await
}

pub async fn load_test_reset(user: &mut GooseUser) -> TransactionResult {
    let payload = json!("");

    make_request(user, GooseMethod::Post, "/reset", "Reset Engine", Some(payload)).await
}

pub async fn load_test_index(user: &mut GooseUser) -> TransactionResult {
    let payload = IndexGlobs {
        globs: vec!["src/*.rs".to_string()],
        chunk_capacity: 0..DEFAULT_CHUNK_CAPACITY,
        chunk_overlap: DEFAULT_CHUNK_OVERLAP,
        chunk_batch_size: DEFAULT_CHUNK_BATCH_SIZE,
    };

    make_request(user, GooseMethod::Post, "/index", "Index Files", Some(payload)).await
}

pub async fn load_test_chat(user: &mut GooseUser) -> TransactionResult {
    let payload = ChatMessages {
        messages: vec![ChatMessage::user("Hello, how are you?".to_string())],
        restart: false,
        persist: false,
    };

    make_request(user, GooseMethod::Post, "/chat", "Chat", Some(payload)).await
}

pub async fn load_test_upload(user: &mut GooseUser) -> TransactionResult {
    let boundary = "----WebKitFormBoundaryABC123";

    // Create a temporary file with some content
    let file_content = "Sample file content for testing";
    let temp_dir = std::env::temp_dir();
    let temp_file_path = temp_dir.join("test.txt");
    std::fs::write(&temp_file_path, file_content).unwrap();

    // Read the file content
    let file_bytes = std::fs::read(&temp_file_path).unwrap();

    // Create the multipart form data
    let mut form_data = Vec::new();

    // Add file part
    form_data.extend_from_slice(format!(
        "--{boundary}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"test.txt\"\r\nContent-Type: text/plain\r\n\r\n",
    ).as_bytes());
    form_data.extend_from_slice(&file_bytes);
    form_data.extend_from_slice(b"\r\n");

    // Add metadata part
    form_data.extend_from_slice(format!(
        "--{boundary}\r\nContent-Disposition: form-data; name=\"metadata\"\r\nContent-Type: application/json\r\n\r\n{{\"path\":\"uploads/test.txt\"}}\r\n",
    ).as_bytes());

    // Add final boundary
    form_data.extend_from_slice(format!("--{boundary}--\r\n").as_bytes());

    let payload = form_data;

    make_request(user, GooseMethod::Post, "/upload", "Upload File", Some(payload)).await
}

pub async fn load_test_delete_source(user: &mut GooseUser) -> TransactionResult {
    let payload = DeleteSource { sources: vec!["uploads/test.txt".to_string()] };

    make_request(user, GooseMethod::Post, "/delete_source", "Delete Source", Some(payload)).await
}

pub async fn load_test_embed(user: &mut GooseUser) -> TransactionResult {
    let payload = json!({
        "model": "nomic-embed-text",
        "input": "Sample text to embed"
    });

    make_request(user, GooseMethod::Post, "/api/embed", "Embed Text", Some(payload)).await
}

pub async fn load_test_ask(user: &mut GooseUser) -> TransactionResult {
    let payload = RetrieveContext {
        query: RetrieveQuery::Text("What is Bioma?".to_string()),
        limit: DEFAULT_RETRIEVER_LIMIT,
        threshold: DEFAULT_RETRIEVER_THRESHOLD,
    };

    make_request(user, GooseMethod::Post, "/ask", "RAG Ask", Some(payload)).await
}

pub async fn load_test_retrieve(user: &mut GooseUser) -> TransactionResult {
    let payload =
        RetrieveContext { query: RetrieveQuery::Text("How to use actors?".to_string()), limit: 5, threshold: 0.0 };

    make_request(user, GooseMethod::Post, "/retrieve", "RAG Retrieve", Some(payload)).await
}

pub async fn load_test_rerank(user: &mut GooseUser) -> TransactionResult {
    let payload = RankTexts {
        query: "What is the weather?".to_string(),
        texts: vec![
            "The weather is sunny today".to_string(),
            "It's raining outside".to_string(),
            "The temperature is 25 degrees".to_string(),
        ],
    };

    make_request(user, GooseMethod::Post, "/rerank", "RAG Rerank", Some(payload)).await
}

#[derive(Debug, Clone, ValueEnum)]
enum TestType {
    Health,
    Hello,
    Index,
    Chat,
    Upload,
    DeleteSource,
    Embed,
    Ask,
    Retrieve,
    Rerank,
    All,
}

#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Args {
    /// Endpoints to run
    #[arg(value_enum, short, long, default_value = "all", value_delimiter = ',')]
    endpoints: Vec<TestType>,

    /// Server URL
    #[arg(short, long, default_value = "http://localhost:5766")]
    server: String,

    /// Number of users
    #[arg(short, long, default_value_t = 10)]
    users: usize,

    /// Run time in seconds
    #[arg(short, long, default_value_t = 60)]
    time: u64,

    /// Request log file path
    #[arg(long, default_value = ".output/requests.log")]
    log: String,

    /// Report file path
    #[arg(long, default_value = ".output/report.html")]
    report: String,

    /// Metrics reporting interval in seconds
    #[arg(short, long, default_value_t = 15)]
    metrics_interval: usize,
}

fn initialize_goose(args: &Args) -> Result<GooseAttack, GooseError> {
    let mut config = GooseConfiguration::default();

    config.host = args.server.clone();
    config.users = Some(args.users);
    config.run_time = args.time.to_string();
    config.request_log = args.log.clone();
    config.report_file = args.report.clone();
    config.running_metrics = Some(args.metrics_interval);

    GooseAttack::initialize_with_config(config)
}

#[tokio::main]
async fn main() -> Result<(), GooseError> {
    let args = Args::parse();
    let mut attack = initialize_goose(&args)?;

    // Helper to register a scenario
    let register_scenario = |attack: GooseAttack, endpoint: TestType| -> Result<GooseAttack, GooseError> {
        match endpoint {
            TestType::Health => Ok(attack.register_scenario(
                scenario!("Health Check").register_transaction(transaction!(load_test_health).set_name("Health Check")),
            )),
            TestType::Hello => Ok(attack.register_scenario(
                scenario!("Hello").register_transaction(transaction!(load_test_hello).set_name("Hello")),
            )),
            TestType::Index => Ok(attack.register_scenario(
                scenario!("Index Files").register_transaction(transaction!(load_test_index).set_name("Index Files")),
            )),
            TestType::Chat => Ok(attack.register_scenario(
                scenario!("Chat").register_transaction(transaction!(load_test_chat).set_name("Chat")),
            )),
            TestType::Upload => Ok(attack.register_scenario(
                scenario!("Upload File").register_transaction(transaction!(load_test_upload).set_name("Upload File")),
            )),
            TestType::DeleteSource => Ok(attack.register_scenario(
                scenario!("Delete Source")
                    .register_transaction(transaction!(load_test_delete_source).set_name("Delete Source")),
            )),
            TestType::Embed => Ok(attack.register_scenario(
                scenario!("Embed Text").register_transaction(transaction!(load_test_embed).set_name("Embed Text")),
            )),
            TestType::Ask => Ok(attack.register_scenario(
                scenario!("RAG Ask").register_transaction(transaction!(load_test_ask).set_name("RAG Ask")),
            )),
            TestType::Retrieve => Ok(attack.register_scenario(
                scenario!("RAG Retrieve")
                    .register_transaction(transaction!(load_test_retrieve).set_name("RAG Retrieve")),
            )),
            TestType::Rerank => Ok(attack.register_scenario(
                scenario!("RAG Rerank").register_transaction(transaction!(load_test_rerank).set_name("RAG Rerank")),
            )),
            TestType::All => Ok(attack.register_scenario(
                scenario!("RAG Server Load Test")
                    .register_transaction(transaction!(load_test_health).set_name("Health Check"))
                    .register_transaction(transaction!(load_test_hello).set_name("Hello"))
                    .register_transaction(transaction!(load_test_index).set_name("Index Files"))
                    .register_transaction(transaction!(load_test_chat).set_name("Chat"))
                    .register_transaction(transaction!(load_test_upload).set_name("Upload File"))
                    .register_transaction(transaction!(load_test_delete_source).set_name("Delete Source"))
                    .register_transaction(transaction!(load_test_embed).set_name("Embed Text"))
                    .register_transaction(transaction!(load_test_ask).set_name("RAG Ask"))
                    .register_transaction(transaction!(load_test_retrieve).set_name("RAG Retrieve"))
                    .register_transaction(transaction!(load_test_rerank).set_name("RAG Rerank")),
            )),
        }
    };

    // Register selected scenarios
    for endpoint in args.endpoints {
        attack = register_scenario(attack, endpoint)?;
    }

    // Execute the attack
    attack.execute().await?;

    Ok(())
}
