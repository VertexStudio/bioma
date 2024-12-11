use bioma_llm::indexer::{DeleteSource, IndexGlobs};
use bioma_llm::prelude::*;
use bioma_llm::rerank::RankTexts;
use bioma_llm::retriever::{RetrieveContext, RetrieveQuery};
use clap::{Parser, ValueEnum};
use goose::config::GooseConfiguration;
use goose::prelude::*;
use once_cell::sync::Lazy;
use rand::Rng;
use serde_json::json;
use std::collections::HashMap;
use std::path::PathBuf;
use std::str::FromStr;
use tokio::fs;
use tokio::io;
use tokio::sync::Mutex;
use tracing::info;

// We will have a global variation count and a global map to track each endpoint's current variation
static VARIATIONS_COUNT: Lazy<Mutex<usize>> = Lazy::new(|| Mutex::new(1));
static ENDPOINT_VARIATION_STATE: Lazy<Mutex<HashMap<TestType, TestVariation>>> =
    Lazy::new(|| Mutex::new(HashMap::new()));

const DEFAULT_CHUNK_CAPACITY: std::ops::Range<usize> = 500..2000;
const DEFAULT_CHUNK_OVERLAP: usize = 200;
const DEFAULT_CHUNK_BATCH_SIZE: usize = 50;

#[derive(Debug, Clone)]
struct TestVariation {
    index: usize,
    file_path: String,
}

impl TestVariation {
    async fn get_new_file_name(&mut self) {
        let file_names = get_test_file_names().await.unwrap();
        println!("Found {} test files", file_names.len());

        // Create a random number generator
        let mut rng = rand::thread_rng();
        let random_number: u32 = rng.gen_range(0..file_names.len() as u32);
        println!("Random number: {}", random_number);

        self.file_path = file_names[random_number as usize].clone()
    }
}

// Global ordering state
static ORDERING_STATE: Lazy<Mutex<OrderingState>> =
    Lazy::new(|| Mutex::new(OrderingState { current_index: 0, endpoint_order: vec![] }));

#[derive(Debug, Clone)]
struct OrderingState {
    current_index: usize,
    endpoint_order: Vec<TestType>,
}

#[derive(Debug, Clone, ValueEnum, PartialEq, Eq, Hash)]
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

// Implement FromStr separately to avoid conflict with ValueEnum
impl FromStr for TestType {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "health" => Ok(TestType::Health),
            "hello" => Ok(TestType::Hello),
            "index" => Ok(TestType::Index),
            "chat" => Ok(TestType::Chat),
            "upload" => Ok(TestType::Upload),
            "deletesource" => Ok(TestType::DeleteSource),
            "embed" => Ok(TestType::Embed),
            "ask" => Ok(TestType::Ask),
            "retrieve" => Ok(TestType::Retrieve),
            "rerank" => Ok(TestType::Rerank),
            "all" => Ok(TestType::All),
            _ => Err(format!("Unknown test type: {}", s)),
        }
    }
}

// Add a helper function to get the next variation index for a given endpoint type
async fn get_next_variation(endpoint_type: TestType) -> TestVariation {
    let variations = *VARIATIONS_COUNT.lock().await;
    let mut map = ENDPOINT_VARIATION_STATE.lock().await;
    let entry = map.entry(endpoint_type).or_insert(TestVariation { index: 0, file_path: "".to_string() });
    entry.get_new_file_name().await;
    println!("Using variation: {}", entry.file_path);
    entry.index = (entry.index + 1) % variations;
    entry.clone()
}

async fn get_test_file_names() -> io::Result<Vec<String>> {
    // Specify the directory path
    let path = "./assets/test_files";
    let mut names = vec![];

    // Read the directory contents
    let mut entries = fs::read_dir(path).await?;

    while let Some(entry) = entries.next_entry().await? {
        let path = entry.path();

        // Check if it's a file
        if path.is_file() {
            names.push(path.to_string_lossy().to_string());
        }
    }

    Ok(names)
}

// Modify initialize_goose to set up initial session data
fn initialize_goose(args: &Args) -> Result<GooseAttack, GooseError> {
    let mut config = GooseConfiguration::default();

    config.host = args.server.clone();
    config.users = Some(args.users);
    config.run_time = args.time.to_string();
    config.request_log = args.log.clone();
    config.report_file = args.report.clone();

    GooseAttack::initialize_with_config(config)
}

// Modify make_request to use global state
async fn make_request<T: serde::Serialize>(
    user: &mut GooseUser,
    method: GooseMethod,
    path: &str,
    name: &str,
    endpoint_type: TestType,
    payload: Option<T>,
) -> TransactionResult {
    // Check ordering using global state but don't update yet
    let mut state = ORDERING_STATE.lock().await;

    let should_execute = {
        info!(
            "Order Check - Current Index: {}, Expected: {:?}, Actual: {:?}",
            state.current_index, state.endpoint_order[state.current_index], endpoint_type
        );
        endpoint_type == state.endpoint_order[state.current_index]
    };

    if !should_execute {
        info!("Skipping {:?} - out of order", endpoint_type);
        return Ok(());
    }

    info!("Executing {:?}", endpoint_type);

    let mut request_builder = user.get_request_builder(&method, path)?;

    if let Some(payload) = payload {
        request_builder = request_builder
            .header("Content-Type", "application/json")
            .body(serde_json::to_string(&payload).unwrap_or_default());
    }

    let goose_request = GooseRequest::builder().set_request_builder(request_builder).name(name).build();

    let mut goose = user.request(goose_request).await?;

    // Update state after request is complete

    let old_index = state.current_index;
    state.current_index = (state.current_index + 1) % state.endpoint_order.len();
    info!("Order Update - Index: {} -> {}, Completed: {:?}", old_index, state.current_index, endpoint_type);

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
    make_request::<()>(user, GooseMethod::Get, "/health", "Health Check", TestType::Health, None).await
}

pub async fn load_test_hello(user: &mut GooseUser) -> TransactionResult {
    let payload = json!("");
    make_request(user, GooseMethod::Post, "/hello", "Hello", TestType::Hello, Some(payload)).await
}

pub async fn load_test_index(user: &mut GooseUser) -> TransactionResult {
    let variation = get_next_variation(TestType::Index).await;
    let file_name = format!("uploads/test{}.txt", variation.file_path);
    let payload = IndexGlobs {
        globs: vec![file_name],
        chunk_capacity: DEFAULT_CHUNK_CAPACITY,
        chunk_overlap: DEFAULT_CHUNK_OVERLAP,
        chunk_batch_size: DEFAULT_CHUNK_BATCH_SIZE,
    };

    make_request(user, GooseMethod::Post, "/index", "Index Files", TestType::Index, Some(payload)).await
}

pub async fn load_test_chat(user: &mut GooseUser) -> TransactionResult {
    let payload = ChatMessages {
        messages: vec![ChatMessage::user("Hello, how are you?".to_string())],
        restart: false,
        persist: false,
    };

    make_request(user, GooseMethod::Post, "/api/chat", "Chat", TestType::Chat, Some(payload)).await
}

pub async fn load_test_upload(user: &mut GooseUser) -> TransactionResult {
    let mut state = ORDERING_STATE.lock().await;
    let should_execute = {
        info!(
            "Upload Order Check - Current Index: {}, Expected: {:?}, Actual: Upload",
            state.current_index, state.endpoint_order[state.current_index]
        );
        TestType::Upload == state.endpoint_order[state.current_index]
    };

    if !should_execute {
        info!("Skipping Upload - out of order");
        return Ok(());
    }

    info!("Executing Upload");

    let variation = get_next_variation(TestType::Upload).await;
    let variation_file_name = PathBuf::from(&variation.file_path).file_name().unwrap().to_string_lossy().to_string();
    dbg!(&variation_file_name);
    let file_name = format!("uploads/stress_tests/{}.md", variation.index);

    let boundary = "----WebKitFormBoundaryABC123";

    // Read the file content
    let file_bytes = std::fs::read(&variation.file_path).unwrap();

    // Create the multipart form data
    let mut form_data = Vec::new();

    // Add file part
    form_data.extend_from_slice(format!(
        "--{boundary}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"{file_name}\"\r\nContent-Type: text/plain\r\n\r\n",
    ).as_bytes());
    form_data.extend_from_slice(&file_bytes);
    form_data.extend_from_slice(b"\r\n");

    // Add metadata part
    form_data.extend_from_slice(format!(
        "--{boundary}\r\nContent-Disposition: form-data; name=\"metadata\"\r\nContent-Type: application/json\r\n\r\n{{\"path\":\"{}\"}}\r\n",
        file_name
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

    // Update state after request is complete

    let old_index = state.current_index;
    state.current_index = (state.current_index + 1) % state.endpoint_order.len();
    info!("Upload Order Update - Index: {} -> {}, Completed: Upload", old_index, state.current_index);

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
    let variation = get_next_variation(TestType::DeleteSource).await;
    let file_name = format!("uploads/test{}.txt", variation.file_path);
    let payload = DeleteSource { sources: vec![file_name] };

    make_request(user, GooseMethod::Post, "/delete_source", "Delete Source", TestType::DeleteSource, Some(payload))
        .await
}

pub async fn load_test_embed(user: &mut GooseUser) -> TransactionResult {
    let payload = json!({
        "model": "nomic-embed-text",
        "input": "Sample text to embed"
    });

    make_request(user, GooseMethod::Post, "/api/embed", "Embed Text", TestType::Embed, Some(payload)).await
}

pub async fn load_test_ask(user: &mut GooseUser) -> TransactionResult {
    let payload = json!({ "query": "What is Bioma?" });

    make_request(user, GooseMethod::Post, "/ask", "RAG Ask", TestType::Ask, Some(payload)).await
}

pub async fn load_test_retrieve(user: &mut GooseUser) -> TransactionResult {
    let payload = RetrieveContext {
        query: RetrieveQuery::Text("How to use actors?".to_string()),
        limit: 5,
        threshold: 0.0,
        source: None,
    };

    make_request(user, GooseMethod::Post, "/retrieve", "RAG Retrieve", TestType::Retrieve, Some(payload)).await
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

    make_request(user, GooseMethod::Post, "/rerank", "RAG Rerank", TestType::Rerank, Some(payload)).await
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

#[derive(Parser, Debug)]
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

    /// Number of variations for the test files
    #[arg(short, long, default_value_t = 1)]
    variations: usize,
}

#[tokio::main]
async fn main() -> Result<(), GooseError> {
    let args = Args::parse();

    // Set the variations count globally
    {
        let mut v = VARIATIONS_COUNT.lock().await;
        *v = args.variations;
    }

    let mut attack = initialize_goose(&args)?;

    // Create a single scenario
    let mut scenario = scenario!("RAG Server Load Test");

    if args.order {
        // Initialize the global ordering state with the endpoint order
        let endpoint_order: Vec<TestType> = args.endpoints.iter().map(|we| we.endpoint.clone()).collect();
        ORDERING_STATE.lock().await.endpoint_order = endpoint_order;
    }

    // Helper to register transactions to the scenario
    let register_transaction = |scenario: Scenario,
                                weighted: WeightedEndpoint,
                                sequence: usize|
     -> Result<Scenario, GooseError> {
        Ok(match weighted.endpoint {
            TestType::Health => scenario.register_transaction(
                transaction!(load_test_health)
                    .set_name("Health Check")
                    .set_sequence(sequence)
                    .set_weight(weighted.weight)?,
            ),
            TestType::Hello => scenario.register_transaction(
                transaction!(load_test_hello).set_name("Hello").set_sequence(sequence).set_weight(weighted.weight)?,
            ),
            TestType::Index => scenario.register_transaction(
                transaction!(load_test_index)
                    .set_name("Index Files")
                    .set_sequence(sequence)
                    .set_weight(weighted.weight)?,
            ),
            TestType::Chat => scenario.register_transaction(
                transaction!(load_test_chat).set_name("Chat").set_sequence(sequence).set_weight(weighted.weight)?,
            ),
            TestType::Upload => scenario.register_transaction(
                transaction!(load_test_upload)
                    .set_name("Upload File")
                    .set_sequence(sequence)
                    .set_weight(weighted.weight)?,
            ),
            TestType::DeleteSource => scenario.register_transaction(
                transaction!(load_test_delete_source)
                    .set_name("Delete Source")
                    .set_sequence(sequence)
                    .set_weight(weighted.weight)?,
            ),
            TestType::Embed => scenario.register_transaction(
                transaction!(load_test_embed)
                    .set_name("Embed Text")
                    .set_sequence(sequence)
                    .set_weight(weighted.weight)?,
            ),
            TestType::Ask => scenario.register_transaction(
                transaction!(load_test_ask).set_name("RAG Ask").set_sequence(sequence).set_weight(weighted.weight)?,
            ),
            TestType::Retrieve => scenario.register_transaction(
                transaction!(load_test_retrieve)
                    .set_name("RAG Retrieve")
                    .set_sequence(sequence)
                    .set_weight(weighted.weight)?,
            ),
            TestType::Rerank => scenario.register_transaction(
                transaction!(load_test_rerank)
                    .set_name("RAG Rerank")
                    .set_sequence(sequence)
                    .set_weight(weighted.weight)?,
            ),
            TestType::All => scenario
                .register_transaction(transaction!(load_test_health).set_name("Health Check").set_sequence(sequence))
                .register_transaction(transaction!(load_test_hello).set_name("Hello").set_sequence(sequence))
                .register_transaction(transaction!(load_test_index).set_name("Index Files").set_sequence(sequence))
                .register_transaction(transaction!(load_test_chat).set_name("Chat").set_sequence(sequence))
                .register_transaction(transaction!(load_test_upload).set_name("Upload File").set_sequence(sequence))
                .register_transaction(
                    transaction!(load_test_delete_source).set_name("Delete Source").set_sequence(sequence),
                )
                .register_transaction(transaction!(load_test_embed).set_name("Embed Text").set_sequence(sequence))
                .register_transaction(transaction!(load_test_ask).set_name("RAG Ask").set_sequence(sequence))
                .register_transaction(transaction!(load_test_retrieve).set_name("RAG Retrieve").set_sequence(sequence))
                .register_transaction(transaction!(load_test_rerank).set_name("RAG Rerank").set_sequence(sequence)),
        })
    };

    // Register selected transactions
    let mut sequence = 1;
    for weighted_endpoint in args.endpoints {
        scenario = register_transaction(scenario, weighted_endpoint, sequence)?;
        sequence += 1;
    }

    // Register the single scenario with all transactions
    attack = attack.register_scenario(scenario);

    // Execute the attack
    attack.execute().await?;

    Ok(())
}
