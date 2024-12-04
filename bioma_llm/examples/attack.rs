use bioma_llm::indexer::{DeleteSource, IndexGlobs};
use bioma_llm::prelude::*;
use bioma_llm::rerank::RankTexts;
use bioma_llm::retriever::{RetrieveContext, RetrieveQuery};
use clap::{Parser, ValueEnum};
use goose::config::GooseConfiguration;
use goose::prelude::*;
use rand::distributions::weighted;
use serde_json::json;
use std::str::FromStr;
use std::sync::Arc;
use tokio::sync::Mutex;

const DEFAULT_CHUNK_CAPACITY: std::ops::Range<usize> = 500..2000;
const DEFAULT_CHUNK_OVERLAP: usize = 200;
const DEFAULT_CHUNK_BATCH_SIZE: usize = 50;

static REQUEST_LOCK: once_cell::sync::Lazy<Arc<Mutex<()>>> = once_cell::sync::Lazy::new(|| Arc::new(Mutex::new(())));

// Add this struct to store our custom session data
#[derive(Debug)]
struct AttackSessionData {
    ordered: bool,
}

// Modify the initialize_goose function to set session data
fn initialize_goose(args: &Args) -> Result<GooseAttack, GooseError> {
    let mut config = GooseConfiguration::default();

    config.host = args.server.clone();
    config.users = Some(args.users);
    config.run_time = args.time.to_string();
    config.request_log = args.log.clone();
    config.report_file = args.report.clone();
    config.running_metrics = Some(args.metrics_interval);

    let mut attack = GooseAttack::initialize_with_config(config)?;

    if args.order {
        attack = attack.set_scheduler(GooseScheduler::Serial);
        // Set session data for all users using test_start
        attack = attack.test_start(transaction!(setup_session_data));
    }

    Ok(attack)
}

// Add this function to set up session data
async fn setup_session_data(user: &mut GooseUser) -> TransactionResult {
    user.set_session_data(AttackSessionData { ordered: true });
    Ok(())
}

// Modify make_request to use session data
async fn make_request<T: serde::Serialize>(
    user: &mut GooseUser,
    method: GooseMethod,
    path: &str,
    name: &str,
    payload: Option<T>,
) -> TransactionResult {
    // Check session data for ordered mode
    let ordered = user.get_session_data::<AttackSessionData>().map(|data| data.ordered).unwrap_or(false);

    // Acquire mutex lock if running in ordered mode
    let _lock = if ordered { Some(REQUEST_LOCK.lock().await) } else { None };

    let mut request_builder = user.get_request_builder(&method, path)?;

    if let Some(payload) = payload {
        request_builder = request_builder
            .header("Content-Type", "application/json")
            .body(serde_json::to_string(&payload).unwrap_or_default());
    }

    let goose_request = GooseRequest::builder().set_request_builder(request_builder).name(name).build();

    let mut goose = user.request(goose_request).await?;

    if let Ok(response) = goose.response {
        if !response.status().is_success() {
            let status = response.status();
            let headers = response.headers().clone();
            let body = response.text().await.unwrap_or_default();
            return user.set_failure(
                &format!("{} request failed with status {}: {}", name.to_lowercase(), status, body),
                &mut goose.request,
                Some(&headers),
                Some(&body),
            );
        }
        goose.response = Ok(response);
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
        globs: vec!["uploads/test.txt".to_string()],
        chunk_capacity: DEFAULT_CHUNK_CAPACITY,
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

    make_request(user, GooseMethod::Post, "/api/chat", "Chat", Some(payload)).await
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

    // Create a custom request with the correct Content-Type header
    let mut request_builder = user.get_request_builder(&GooseMethod::Post, "/upload")?;
    request_builder =
        request_builder.header("Content-Type", format!("multipart/form-data; boundary={}", boundary)).body(payload);

    let goose_request = GooseRequest::builder().set_request_builder(request_builder).name("Upload File").build();

    let mut goose = user.request(goose_request).await?;

    if let Ok(response) = goose.response {
        if !response.status().is_success() {
            let status = response.status();
            let headers = response.headers().clone();
            let body = response.text().await.unwrap_or_default();
            return user.set_failure(
                &format!("upload file request failed with status {}: {}", status, body),
                &mut goose.request,
                Some(&headers),
                Some(&body),
            );
        }
        goose.response = Ok(response);
    }

    Ok(())
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
    let payload = json!({ "query": "What is Bioma?" });

    make_request(user, GooseMethod::Post, "/ask", "RAG Ask", Some(payload)).await
}

pub async fn load_test_retrieve(user: &mut GooseUser) -> TransactionResult {
    let payload = RetrieveContext {
        query: RetrieveQuery::Text("How to use actors?".to_string()),
        limit: 5,
        threshold: 0.0,
        source: None,
    };

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
    // Hello,
    Index,
    // Chat,
    // Upload,
    // DeleteSource,
    // Embed,
    // Ask,
    // Retrieve,
    // Rerank,
    // All,
}

// Implement FromStr separately to avoid conflict with ValueEnum
impl FromStr for TestType {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "health" => Ok(TestType::Health),
            // "hello" => Ok(TestType::Hello),
            "index" => Ok(TestType::Index),
            // "chat" => Ok(TestType::Chat),
            // "upload" => Ok(TestType::Upload),
            // "deletesource" => Ok(TestType::DeleteSource),
            // "embed" => Ok(TestType::Embed),
            // "ask" => Ok(TestType::Ask),
            // "retrieve" => Ok(TestType::Retrieve),
            // "rerank" => Ok(TestType::Rerank),
            // "all" => Ok(TestType::All),
            _ => Err(format!("Unknown test type: {}", s)),
        }
    }
}

#[derive(Parser, Debug, Clone)]
#[command(author, version, about, long_about = None)]
struct Args {
    /// Endpoints to run with optional weights (endpoint:weight,endpoint:weight)
    #[arg(value_delimiter = ',', short, long, default_value = "all")]
    endpoints: Vec<WeightedEndpoint>,

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

    /// Run scenarios in sequential order
    #[arg(long)]
    order: bool,
}

#[derive(Debug, Clone)]
struct WeightedEndpoint {
    endpoint: TestType,
    weight: usize,
}

impl FromStr for WeightedEndpoint {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let parts: Vec<&str> = s.split(':').collect();
        match parts.as_slice() {
            [endpoint, weight] => {
                let endpoint =
                    <TestType as FromStr>::from_str(endpoint).map_err(|_| format!("Invalid endpoint: {}", endpoint))?;
                let weight = weight.parse().map_err(|_| format!("Invalid weight: {}", weight))?;
                Ok(WeightedEndpoint { endpoint, weight })
            }
            [endpoint] => {
                let endpoint =
                    <TestType as FromStr>::from_str(endpoint).map_err(|_| format!("Invalid endpoint: {}", endpoint))?;
                Ok(WeightedEndpoint { endpoint, weight: 1 })
            }
            _ => Err(format!("Invalid format: {}", s)),
        }
    }
}

#[tokio::main]
async fn main() -> Result<(), GooseError> {
    let args = Args::parse();
    dbg!(&args);

    for weighted_endpoint in args.clone().endpoints {
        match weighted_endpoint.endpoint {
            TestType::Health => {
                let health_metrics = initialize_goose(&args)?
                    .register_scenario(
                        scenario!("Health Check").register_transaction(
                            transaction!(load_test_health).set_name("Health Check").set_weight(5)?,
                        ),
                        //.register_transaction(transaction!(load_test_hello).set_name("Hello").set_weight(2)?),
                    )
                    .execute()
                    .await?;

                assert!(health_metrics.errors.is_empty());
            }
            TestType::Index => {
                let upload_metrics = initialize_goose(&args)?
                    .register_scenario(
                        scenario!("Upload Check")
                            .register_transaction(transaction!(load_test_upload).set_name("Upload Check").set_weight(5)?),
                    )
                    .execute()
                    .await?;

                assert!(upload_metrics.errors.is_empty());
            }
        }
    }

    // let index_metrics = initialize_goose(&args)?
    //     .register_scenario(
    //         scenario!("Index Check")
    //             .register_transaction(transaction!(load_test_index).set_name("Index Check").set_weight(5)?),
    //     )
    //     .execute()
    //     .await?;

    // assert!(index_metrics.errors.is_empty());

    // let embed_metrics = initialize_goose(&args)?
    //     .register_scenario(
    //         scenario!("Embed Check")
    //             .register_transaction(transaction!(load_test_embed).set_name("Embed Check").set_weight(5)?),
    //     )
    //     .execute()
    //     .await?;

    // assert!(embed_metrics.errors.is_empty());

    // let read_metrics = initialize_goose(&args)?
    //     .register_scenario(
    //         scenario!("Read Check")
    //             .register_transaction(transaction!(load_test_chat).set_name("Chat Check").set_weight(5)?)
    //             .register_transaction(transaction!(load_test_ask).set_name("Ask Check").set_weight(5)?)
    //             .register_transaction(transaction!(load_test_retrieve).set_name("Retrieve Check").set_weight(5)?)
    //             .register_transaction(transaction!(load_test_rerank).set_name("Rerank Check").set_weight(5)?),
    //     )
    //     .execute()
    //     .await?;

    // assert!(read_metrics.errors.is_empty());

    // let read_metrics = initialize_goose(&args)?
    //     .register_scenario(
    //         scenario!("Delete Check")
    //             .register_transaction(transaction!(load_test_delete_source).set_name("Delete Check").set_weight(5)?)
    //     )
    //     .execute()
    //     .await?;

    // assert!(read_metrics.errors.is_empty());

    Ok(())
}
