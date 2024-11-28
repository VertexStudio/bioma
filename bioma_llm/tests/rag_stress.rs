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

    let goose_request = GooseRequest::builder().set_request_builder(request_builder).build();

    let _goose = user.request(goose_request).await?;

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

    let goose_request = GooseRequest::builder().set_request_builder(request_builder).build();

    let _goose = user.request(goose_request).await?;

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

    let goose_request = GooseRequest::builder().set_request_builder(request_builder).build();

    let _goose = user.request(goose_request).await?;

    Ok(())
}

#[tokio::test]
async fn test_stress_rag_server() -> Result<(), GooseError> {
    GooseAttack::initialize()?
        .set_default(GooseDefault::Host, "http://localhost:8080")?
        .set_default(GooseDefault::Users, 10)?
        .set_default(GooseDefault::RunTime, 60)?
        .set_default(GooseDefault::RequestLog, "../.output/requests.log")?
        .register_scenario(
            scenario!("LoadTest")
                .register_transaction(transaction!(load_test_ask))
                .register_transaction(transaction!(load_test_retrieve))
                .register_transaction(transaction!(load_test_rerank)),
        )
        .execute()
        .await?;

    Ok(())
}
