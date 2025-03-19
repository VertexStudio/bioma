use bioma_llm::prelude::*;
use bioma_rag::prelude::*;
use clap::{Parser, ValueEnum};
use goose::config::GooseConfiguration;
use goose::prelude::*;
use once_cell::sync::Lazy;
use rand::Rng;
use serde_json::json;
use std::str::FromStr;
use tokio::fs;
use tokio::io;
use tokio::sync::Mutex;
use tracing::info;

// We will have a global variation count and a global map to track each endpoint's current variation
static VARIATIONS_COUNT: Lazy<Mutex<usize>> = Lazy::new(|| Mutex::new(1));
static VARIATION_STATE: Lazy<Mutex<TestVariation>> =
    Lazy::new(|| Mutex::new(TestVariation { index: 0, file_path: "".to_string() }));

#[derive(Debug, Clone)]
struct TestVariation {
    index: usize,
    file_path: String,
}

impl TestVariation {
    async fn set_new_random_file_name(&mut self) {
        let file_names = get_test_file_names().await.unwrap();

        // Create a random number generator
        let mut rng = rand::thread_rng();
        let random_number: u32 = rng.gen_range(0..file_names.len() as u32);

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
    Delete,
    Embed,
    Ask,
    Retrieve,
    Rerank,
    All,
}

impl TestType {
    fn all_order() -> Vec<TestType> {
        vec![
            TestType::Health,
            TestType::Hello,
            TestType::Upload,
            TestType::Index,
            TestType::Retrieve,
            TestType::Ask,
            TestType::Chat,
            TestType::Delete,
            TestType::Embed,
            TestType::Rerank,
        ]
    }
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
            "delete" => Ok(TestType::Delete),
            "embed" => Ok(TestType::Embed),
            "ask" => Ok(TestType::Ask),
            "retrieve" => Ok(TestType::Retrieve),
            "rerank" => Ok(TestType::Rerank),
            "all" => Ok(TestType::All),
            _ => Err(format!("Unknown test type: {}", s)),
        }
    }
}

/// Add a helper function to get the next variation index for a given endpoint type
async fn get_next_variation(
    endpoint_type: TestType,
    variation_state: &mut tokio::sync::MutexGuard<'_, TestVariation>,
    variations: usize,
    ordering_state: &mut tokio::sync::MutexGuard<'_, OrderingState>,
) -> TestVariation {
    // If the array is empty (no --order flag), always get a new variation, if is not empty, will check for the following:
    // If the current endpoint is the first in the list AND it is the time to execute it, change the variation,
    // otherwise, return current variation
    if ordering_state.endpoint_order.is_empty()
        || (ordering_state.endpoint_order[0].eq(&endpoint_type) && should_execute(&endpoint_type, ordering_state).await)
    {
        // Get the current variation for the endpoint type or create a new one if it does not exist and set a new random file name for the variation
        variation_state.set_new_random_file_name().await;

        // This ensures that the index is always within 0 and (variations - 1), ensuring the exact variations
        variation_state.index = (variation_state.index + 1) % variations;

        variation_state.clone()
    } else {
        variation_state.clone()
    }
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
    config.report_file = vec![args.report.clone()];
    config.running_metrics = Some(args.metrics_interval);

    GooseAttack::initialize_with_config(config)
}

/// Check if the endpoint_type should execute, according to the ordering state
async fn should_execute(
    endpoint_type: &TestType,
    ordering_state: &mut tokio::sync::MutexGuard<'_, OrderingState>,
) -> bool {
    if ordering_state.endpoint_order.is_empty() {
        return true;
    }

    info!(
        "Order Check - Current Index: {}, Expected: {:?}, Actual: {:?}",
        ordering_state.current_index, ordering_state.endpoint_order[ordering_state.current_index], endpoint_type
    );
    endpoint_type == &ordering_state.endpoint_order[ordering_state.current_index]
}

/// Modify make_request to use global state
async fn make_request<T: serde::Serialize>(
    user: &mut GooseUser,
    method: GooseMethod,
    path: &str,
    name: &str,
    endpoint_type: TestType,
    payload: Option<T>,
    ordering_state: &mut tokio::sync::MutexGuard<'_, OrderingState>,
) -> TransactionResult {
    let should_execute = should_execute(&endpoint_type, ordering_state).await;

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

    // Update ordering_state after request is complete
    if !ordering_state.endpoint_order.is_empty() {
        let old_index = ordering_state.current_index;
        ordering_state.current_index = (ordering_state.current_index + 1) % ordering_state.endpoint_order.len();

        info!(
            "Order Update - Index: {} -> {}, Completed: {:?}",
            old_index, ordering_state.current_index, endpoint_type
        );
    }

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
    let mut variation_state: tokio::sync::MutexGuard<'_, TestVariation> = VARIATION_STATE.lock().await;
    let variations = *VARIATIONS_COUNT.lock().await;
    let mut ordering_state = ORDERING_STATE.lock().await;

    get_next_variation(TestType::Health, &mut variation_state, variations, &mut ordering_state).await;

    make_request::<()>(user, GooseMethod::Get, "/health", "Health Check", TestType::Health, None, &mut ordering_state)
        .await
}

pub async fn load_test_hello(user: &mut GooseUser) -> TransactionResult {
    let mut variation_state: tokio::sync::MutexGuard<'_, TestVariation> = VARIATION_STATE.lock().await;
    let variations = *VARIATIONS_COUNT.lock().await;
    let mut ordering_state = ORDERING_STATE.lock().await;

    get_next_variation(TestType::Hello, &mut variation_state, variations, &mut ordering_state).await;

    let payload = json!("");
    make_request(user, GooseMethod::Post, "/hello", "Hello", TestType::Hello, Some(payload), &mut ordering_state).await
}

pub async fn load_test_index(user: &mut GooseUser) -> TransactionResult {
    let mut variation_state: tokio::sync::MutexGuard<'_, TestVariation> = VARIATION_STATE.lock().await;
    let variations = *VARIATIONS_COUNT.lock().await;
    let mut ordering_state = ORDERING_STATE.lock().await;

    let variation = get_next_variation(TestType::Index, &mut variation_state, variations, &mut ordering_state).await;

    let file_name = format!("uploads/stress_tests/{}.md", variation.index);
    let payload =
        Index::builder().content(IndexContent::Globs(GlobsContent::builder().globs(vec![file_name]).build())).build();

    make_request(user, GooseMethod::Post, "/index", "Index Files", TestType::Index, Some(payload), &mut ordering_state)
        .await
}

pub async fn load_test_chat(user: &mut GooseUser) -> TransactionResult {
    let mut variation_state: tokio::sync::MutexGuard<'_, TestVariation> = VARIATION_STATE.lock().await;
    let variations = *VARIATIONS_COUNT.lock().await;
    let mut ordering_state = ORDERING_STATE.lock().await;

    get_next_variation(TestType::Chat, &mut variation_state, variations, &mut ordering_state).await;

    let payload = json!({
        "messages": [ChatMessage::user("Hello, how are you?".to_string())],
    });

    make_request(user, GooseMethod::Post, "/chat", "Chat", TestType::Chat, Some(payload), &mut ordering_state).await
}

pub async fn load_test_upload(user: &mut GooseUser) -> TransactionResult {
    let mut variation_state: tokio::sync::MutexGuard<'_, TestVariation> = VARIATION_STATE.lock().await;
    let variations = *VARIATIONS_COUNT.lock().await;
    let mut ordering_state = ORDERING_STATE.lock().await;

    let variation = get_next_variation(TestType::Upload, &mut variation_state, variations, &mut ordering_state).await;

    let should_execute = should_execute(&TestType::Upload, &mut ordering_state).await;

    if !should_execute {
        info!("Skipping Upload - out of order");
        return Ok(());
    }

    info!("Executing Upload");

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

    if !ordering_state.endpoint_order.is_empty() {
        let old_index = ordering_state.current_index;
        ordering_state.current_index = (ordering_state.current_index + 1) % ordering_state.endpoint_order.len();
        info!("Upload Order Update - Index: {} -> {}, Completed: Upload", old_index, ordering_state.current_index);
    }

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
    let mut variation_state: tokio::sync::MutexGuard<'_, TestVariation> = VARIATION_STATE.lock().await;
    let variations = *VARIATIONS_COUNT.lock().await;
    let mut ordering_state = ORDERING_STATE.lock().await;

    let variation = get_next_variation(TestType::Delete, &mut variation_state, variations, &mut ordering_state).await;

    let file_name = format!("uploads/stress_tests/{}.md", variation.index);
    let payload = DeleteSource { sources: vec![file_name], delete_from_disk: true };

    make_request(
        user,
        GooseMethod::Post,
        "/delete_source",
        "Delete Source",
        TestType::Delete,
        Some(payload),
        &mut ordering_state,
    )
    .await
}

pub async fn load_test_embed(user: &mut GooseUser) -> TransactionResult {
    let mut variation_state: tokio::sync::MutexGuard<'_, TestVariation> = VARIATION_STATE.lock().await;
    let variations = *VARIATIONS_COUNT.lock().await;
    let mut ordering_state = ORDERING_STATE.lock().await;

    let variation = get_next_variation(TestType::Embed, &mut variation_state, variations, &mut ordering_state).await;

    // Read the file content
    let file_bytes = std::fs::read(&variation.file_path).unwrap();

    let payload = json!({
        "model": "nomic-embed-text",
        "input": String::from_utf8_lossy(&file_bytes).to_string(),
    });

    make_request(user, GooseMethod::Post, "/embed", "Embed Text", TestType::Embed, Some(payload), &mut ordering_state)
        .await
}

pub async fn load_test_ask(user: &mut GooseUser) -> TransactionResult {
    let mut variation_state: tokio::sync::MutexGuard<'_, TestVariation> = VARIATION_STATE.lock().await;
    let variations = *VARIATIONS_COUNT.lock().await;
    let mut ordering_state = ORDERING_STATE.lock().await;

    get_next_variation(TestType::Ask, &mut variation_state, variations, &mut ordering_state).await;

    let payload = json!({
        "messages": [ChatMessage::user("What is ubuntu, surrealdb and rust? Please divide the answer in sections.".to_string())],
    });

    make_request(user, GooseMethod::Post, "/ask", "RAG Ask", TestType::Ask, Some(payload), &mut ordering_state).await
}

pub async fn load_test_retrieve(user: &mut GooseUser) -> TransactionResult {
    let mut variation_state: tokio::sync::MutexGuard<'_, TestVariation> = VARIATION_STATE.lock().await;
    let variations = *VARIATIONS_COUNT.lock().await;
    let mut ordering_state = ORDERING_STATE.lock().await;

    get_next_variation(TestType::Retrieve, &mut variation_state, variations, &mut ordering_state).await;

    let payload = RetrieveContext::builder()
        .query(RetrieveQuery::Text("Explain About ubuntu, surrealdb and Rust?".to_string()))
        .limit(5)
        .threshold(0.0)
        .build();

    make_request(
        user,
        GooseMethod::Post,
        "/retrieve",
        "RAG Retrieve",
        TestType::Retrieve,
        Some(payload),
        &mut ordering_state,
    )
    .await
}

pub async fn load_test_rerank(user: &mut GooseUser) -> TransactionResult {
    let mut variation_state: tokio::sync::MutexGuard<'_, TestVariation> = VARIATION_STATE.lock().await;
    let variations = *VARIATIONS_COUNT.lock().await;
    let mut ordering_state = ORDERING_STATE.lock().await;

    get_next_variation(TestType::Rerank, &mut variation_state, variations, &mut ordering_state).await;

    let payload = RankTexts::builder()
        .query("Explain About ubuntu, surrealdb and Rust?".to_string())
        .texts(vec![
            "The weather is sunny today".to_string(),
            "It's raining outside".to_string(),
            "The temperature is 25 degrees".to_string(),
        ])
        .build();

    make_request(user, GooseMethod::Post, "/rerank", "RAG Rerank", TestType::Rerank, Some(payload), &mut ordering_state)
        .await
}

#[derive(Debug, Clone, PartialEq)]
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

/// Cli tool to run stress test to the RAG server.
/// To see all the available endpoints for --endpoints, please visit the README at bioma_llm crate.
#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Args {
    /// Endpoints to run with optional weights (endpoint:weight,endpoint:weight,endpoint).
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
    #[arg(short, long, default_value_t = 0)]
    metrics_interval: usize,

    /// Run scenarios in sequential order
    #[arg(long)]
    order: bool,

    /// Number of variations for the test files
    #[arg(short, long, default_value_t = 5)]
    variations: usize,
}

fn contains_endpoint(endpoints: &[WeightedEndpoint], target: TestType) -> bool {
    endpoints.iter().any(|e| e.endpoint == target)
}

#[tokio::main]
async fn main() -> Result<(), GooseError> {
    let args = Args::parse();

    if contains_endpoint(&args.endpoints, TestType::All) && args.endpoints.len() > 1 {
        panic!("\"all\" endpoint should be used alone");
    }

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

        if args.endpoints.len() == 1 && args.endpoints[0].endpoint == TestType::All {
            ORDERING_STATE.lock().await.endpoint_order = TestType::all_order();
        } else {
            let endpoint_order: Vec<TestType> = args.endpoints.iter().map(|we| we.endpoint.clone()).collect();
            ORDERING_STATE.lock().await.endpoint_order = endpoint_order;
        }
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
            TestType::Delete => scenario.register_transaction(
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
