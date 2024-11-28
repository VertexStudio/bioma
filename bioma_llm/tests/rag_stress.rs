use goose::prelude::*;
use serde_json::json;

async fn load_test_ask(user: &mut GooseUser) -> TransactionResult {
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

async fn load_test_retrieve(user: &mut GooseUser) -> TransactionResult {
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

async fn load_test_rerank(user: &mut GooseUser) -> TransactionResult {
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

#[tokio::test]
async fn test_stress_rag_server() -> Result<(), GooseError> {
    GooseAttack::initialize()?
        .set_default(GooseDefault::Host, "http://localhost:8080")?
        .set_default(GooseDefault::Users, 10)?
        .set_default(GooseDefault::RunTime, 60)?
        .set_default(GooseDefault::RequestLog, "../.output/requests.log")?
        .set_default(GooseDefault::ReportFile, "../.output/report.html")?
        .set_default(GooseDefault::RunningMetrics, 15)?
        .register_scenario(
            scenario!("RAG Load Test")
                .register_transaction(transaction!(load_test_ask).set_name("Ask").set_weight(3)?)
                .register_transaction(transaction!(load_test_retrieve).set_name("Retrieve").set_weight(2)?)
                .register_transaction(transaction!(load_test_rerank).set_name("Rerank").set_weight(1)?),
        )
        .execute()
        .await?;

    Ok(())
}
