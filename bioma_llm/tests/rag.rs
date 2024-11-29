use goose::config::GooseConfiguration;
use goose::prelude::*;
use goose::GooseError;
use serde_json::json;
use test_log::test;

pub async fn load_test_health(user: &mut GooseUser) -> TransactionResult {
    let request_builder = user.get_request_builder(&GooseMethod::Get, "/health")?.header("Accept", "text/plain");

    let goose_request = GooseRequest::builder().set_request_builder(request_builder).name("Health Check").build();

    let mut goose = user.request(goose_request).await?;

    if let Ok(response) = &goose.response {
        if !response.status().is_success() {
            return user.set_failure("health check failed", &mut goose.request, Some(response.headers()), None);
        }
    }

    Ok(())
}

pub async fn load_test_hello(user: &mut GooseUser) -> TransactionResult {
    let payload = json!("");

    let request_builder = user
        .get_request_builder(&GooseMethod::Post, "/hello")?
        .header("Content-Type", "application/json")
        .body(payload.to_string());

    let goose_request = GooseRequest::builder().set_request_builder(request_builder).name("Hello").build();

    let mut goose = user.request(goose_request).await?;

    if let Ok(response) = &goose.response {
        if !response.status().is_success() {
            return user.set_failure("hello request failed", &mut goose.request, Some(response.headers()), None);
        }
    }

    Ok(())
}

pub async fn load_test_reset(user: &mut GooseUser) -> TransactionResult {
    let request_builder = user.get_request_builder(&GooseMethod::Post, "/reset")?.header("Accept", "text/plain");

    let goose_request = GooseRequest::builder().set_request_builder(request_builder).name("Reset Engine").build();

    let mut goose = user.request(goose_request).await?;

    if let Ok(response) = &goose.response {
        if !response.status().is_success() {
            return user.set_failure("reset request failed", &mut goose.request, Some(response.headers()), None);
        }
    }

    Ok(())
}

pub async fn load_test_index(user: &mut GooseUser) -> TransactionResult {
    let payload = json!({
        "globs": ["/path/to/your/files/**/*.rs"] // Update the path as needed
    });

    let request_builder = user
        .get_request_builder(&GooseMethod::Post, "/index")?
        .header("Content-Type", "application/json")
        .body(payload.to_string());

    let goose_request = GooseRequest::builder().set_request_builder(request_builder).name("Index Files").build();

    let mut goose = user.request(goose_request).await?;

    if let Ok(response) = &goose.response {
        if !response.status().is_success() {
            return user.set_failure("index request failed", &mut goose.request, Some(response.headers()), None);
        }
    }

    Ok(())
}

pub async fn load_test_chat(user: &mut GooseUser) -> TransactionResult {
    let payload = json!({
        "messages": [
            {
                "role": "user",
                "content": "Hello, how are you?"
            }
        ]
    });

    let request_builder = user
        .get_request_builder(&GooseMethod::Post, "/chat")?
        .header("Content-Type", "application/json")
        .body(payload.to_string());

    let goose_request = GooseRequest::builder().set_request_builder(request_builder).name("Chat").build();

    let mut goose = user.request(goose_request).await?;

    if let Ok(response) = &goose.response {
        if !response.status().is_success() {
            return user.set_failure("chat request failed", &mut goose.request, Some(response.headers()), None);
        }
    }

    Ok(())
}

pub async fn load_test_upload(user: &mut GooseUser) -> TransactionResult {
    let boundary = "----WebKitFormBoundary7MA4YWxkTrZu0gW";
    let file_content = "Sample file content"; // Replace with actual file content or load from a fixture

    let payload = format!(
        "--{}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"test.txt\"\r\nContent-Type: text/plain\r\n\r\n{}\r\n--{}\r\nContent-Disposition: form-data; name=\"metadata\"\r\nContent-Type: application/json\r\n\r\n{{\"path\": \"uploads/test.txt\"}}\r\n--{}--\r\n",
        boundary, file_content, boundary, boundary
    );

    let request_builder = user
        .get_request_builder(&GooseMethod::Post, "/upload")?
        .header("Content-Type", format!("multipart/form-data; boundary={}", boundary))
        .body(payload);

    let goose_request = GooseRequest::builder().set_request_builder(request_builder).name("Upload File").build();

    let mut goose = user.request(goose_request).await?;

    if let Ok(response) = &goose.response {
        if !response.status().is_success() {
            return user.set_failure("upload request failed", &mut goose.request, Some(response.headers()), None);
        }
    }

    Ok(())
}

pub async fn load_test_delete_source(user: &mut GooseUser) -> TransactionResult {
    let payload = json!({
        "sources": ["uploads/test.txt"] // Update with actual sources to delete
    });

    let request_builder = user
        .get_request_builder(&GooseMethod::Post, "/delete_source")?
        .header("Content-Type", "application/json")
        .body(payload.to_string());

    let goose_request = GooseRequest::builder().set_request_builder(request_builder).name("Delete Source").build();

    let mut goose = user.request(goose_request).await?;

    if let Ok(response) = &goose.response {
        if !response.status().is_success() {
            return user.set_failure(
                "delete_source request failed",
                &mut goose.request,
                Some(response.headers()),
                None,
            );
        }
    }

    Ok(())
}

pub async fn load_test_embed(user: &mut GooseUser) -> TransactionResult {
    let payload = json!({
        "model": "nomic-embed-text",
        "input": "Sample text to embed"
    });

    let request_builder = user
        .get_request_builder(&GooseMethod::Post, "/api/embed")?
        .header("Content-Type", "application/json")
        .body(payload.to_string());

    let goose_request = GooseRequest::builder().set_request_builder(request_builder).name("Embed Text").build();

    let mut goose = user.request(goose_request).await?;

    if let Ok(response) = &goose.response {
        if !response.status().is_success() {
            return user.set_failure("embed request failed", &mut goose.request, Some(response.headers()), None);
        }
    }

    Ok(())
}

pub async fn load_test_ask(user: &mut GooseUser) -> TransactionResult {
    let payload = json!({
        "query": "What is Bioma?",
    });

    let request_builder = user
        .get_request_builder(&GooseMethod::Post, "/ask")?
        .header("Content-Type", "application/json")
        .body(payload.to_string());

    let goose_request = GooseRequest::builder().set_request_builder(request_builder).name("RAG Ask").build();

    let mut goose = user.request(goose_request).await?;

    if let Ok(response) = &goose.response {
        if !response.status().is_success() {
            return user.set_failure("ask request failed", &mut goose.request, Some(response.headers()), None);
        }
    }

    Ok(())
}

pub async fn load_test_retrieve(user: &mut GooseUser) -> TransactionResult {
    let payload = json!({
        "query": "How to use actors?",
        "limit": 5,
        "threshold": 0.0
    });

    let request_builder = user
        .get_request_builder(&GooseMethod::Post, "/retrieve")?
        .header("Content-Type", "application/json")
        .body(payload.to_string());

    let goose_request = GooseRequest::builder().set_request_builder(request_builder).name("RAG Retrieve").build();

    let mut goose = user.request(goose_request).await?;

    if let Ok(response) = &goose.response {
        if !response.status().is_success() {
            return user.set_failure("retrieve request failed", &mut goose.request, Some(response.headers()), None);
        }
    }

    Ok(())
}

pub async fn load_test_rerank(user: &mut GooseUser) -> TransactionResult {
    let payload = json!({
        "query": "What is the weather?",
        "texts": [
            "The weather is sunny today",
            "It's raining outside",
            "The temperature is 25 degrees"
        ]
    });

    let request_builder = user
        .get_request_builder(&GooseMethod::Post, "/rerank")?
        .header("Content-Type", "application/json")
        .body(payload.to_string());

    let goose_request = GooseRequest::builder().set_request_builder(request_builder).name("RAG Rerank").build();

    let mut goose = user.request(goose_request).await?;

    if let Ok(response) = &goose.response {
        if !response.status().is_success() {
            return user.set_failure("rerank request failed", &mut goose.request, Some(response.headers()), None);
        }
    }

    Ok(())
}

// Helper function to initialize GooseAttack with common settings
fn initialize_goose() -> Result<GooseAttack, GooseError> {
    let mut config = GooseConfiguration::default();

    config.host = "http://localhost:8080".to_string();
    config.users = Some(10);
    config.run_time = "60".to_string();
    config.request_log = "../.output/requests.log".to_string();
    config.report_file = "../.output/report.html".to_string();
    config.running_metrics = Some(15);

    // Initialize GooseAttack with the configuration, otherwise it'll load cli args and since this is a test, we don't want that
    GooseAttack::initialize_with_config(config)
}

#[test(tokio::test)]
async fn test_load_health() -> Result<(), GooseError> {
    initialize_goose()?
        .register_scenario(
            scenario!("Health Check")
                .register_transaction(transaction!(load_test_health).set_name("Health Check").set_weight(5)?),
        )
        .execute()
        .await?;

    Ok(())
}

#[test(tokio::test)]
async fn test_load_hello() -> Result<(), GooseError> {
    initialize_goose()?
        .register_scenario(
            scenario!("Hello").register_transaction(transaction!(load_test_hello).set_name("Hello").set_weight(2)?),
        )
        .execute()
        .await?;

    Ok(())
}

#[test(tokio::test)]
async fn test_load_index() -> Result<(), GooseError> {
    initialize_goose()?
        .register_scenario(
            scenario!("Index Files")
                .register_transaction(transaction!(load_test_index).set_name("Index Files").set_weight(3)?),
        )
        .execute()
        .await?;

    Ok(())
}

#[test(tokio::test)]
async fn test_load_chat() -> Result<(), GooseError> {
    initialize_goose()?
        .register_scenario(
            scenario!("Chat").register_transaction(transaction!(load_test_chat).set_name("Chat").set_weight(4)?),
        )
        .execute()
        .await?;

    Ok(())
}

#[test(tokio::test)]
async fn test_load_delete_source() -> Result<(), GooseError> {
    initialize_goose()?
        .register_scenario(
            scenario!("Delete Source")
                .register_transaction(transaction!(load_test_delete_source).set_name("Delete Source").set_weight(1)?),
        )
        .execute()
        .await?;

    Ok(())
}

#[test(tokio::test)]
async fn test_load_embed() -> Result<(), GooseError> {
    initialize_goose()?
        .register_scenario(
            scenario!("Embed Text")
                .register_transaction(transaction!(load_test_embed).set_name("Embed Text").set_weight(3)?),
        )
        .execute()
        .await?;

    Ok(())
}

#[test(tokio::test)]
async fn test_load_ask() -> Result<(), GooseError> {
    initialize_goose()?
        .register_scenario(
            scenario!("RAG Ask").register_transaction(transaction!(load_test_ask).set_name("RAG Ask").set_weight(3)?),
        )
        .execute()
        .await?;

    Ok(())
}

#[test(tokio::test)]
async fn test_load_retrieve() -> Result<(), GooseError> {
    initialize_goose()?
        .register_scenario(
            scenario!("RAG Retrieve")
                .register_transaction(transaction!(load_test_retrieve).set_name("RAG Retrieve").set_weight(2)?),
        )
        .execute()
        .await?;

    Ok(())
}

#[test(tokio::test)]
async fn test_load_rerank() -> Result<(), GooseError> {
    initialize_goose()?
        .register_scenario(
            scenario!("RAG Rerank")
                .register_transaction(transaction!(load_test_rerank).set_name("RAG Rerank").set_weight(1)?),
        )
        .execute()
        .await?;

    Ok(())
}

#[test(tokio::test)]
async fn test_load_rag_server() -> Result<(), GooseError> {
    initialize_goose()?
        .register_scenario(
            scenario!("RAG Server Load Test")
                .register_transaction(transaction!(load_test_health).set_name("Health Check").set_weight(5)?) // Higher weight for health checks
                .register_transaction(transaction!(load_test_hello).set_name("Hello").set_weight(2)?)
                .register_transaction(transaction!(load_test_index).set_name("Index Files").set_weight(3)?)
                .register_transaction(transaction!(load_test_chat).set_name("Chat").set_weight(4)?)
                // .register_transaction(transaction!(load_test_upload).set_name("Upload File").set_weight(1)?)
                .register_transaction(transaction!(load_test_delete_source).set_name("Delete Source").set_weight(1)?)
                .register_transaction(transaction!(load_test_embed).set_name("Embed Text").set_weight(3)?)
                .register_transaction(transaction!(load_test_ask).set_name("RAG Ask").set_weight(3)?)
                .register_transaction(transaction!(load_test_retrieve).set_name("RAG Retrieve").set_weight(2)?)
                .register_transaction(transaction!(load_test_rerank).set_name("RAG Rerank").set_weight(1)?),
        )
        .execute()
        .await?;

    Ok(())
}
