use actix_web::web::Bytes;
use futures::future::join_all;
use reqwest::Client;
use serde_json::json;
use std::time::Instant;
use tokio::time::Duration;

#[derive(Debug)]
struct StressTestMetrics {
    endpoint: String,
    requests: usize,
    success_count: usize,
    failure_count: usize,
    avg_response_time: f64,
    min_response_time: Duration,
    max_response_time: Duration,
}

async fn stress_test_endpoint(
    client: &Client,
    endpoint: &str,
    payload: serde_json::Value,
    num_requests: usize,
    concurrent_requests: usize,
) -> StressTestMetrics {
    let base_url = "http://localhost:8080";
    let url = format!("{}{}", base_url, endpoint);

    let mut response_times = Vec::with_capacity(num_requests);
    let mut success_count = 0;
    let mut failure_count = 0;

    for chunk in (0..num_requests).collect::<Vec<_>>().chunks(concurrent_requests) {
        let requests = chunk.iter().map(|_| {
            let client = client.clone();
            let url = url.clone();
            let payload = payload.clone();

            async move {
                let start = Instant::now();
                let result = client.post(&url).json(&payload).send().await;
                let duration = start.elapsed();

                (result, duration)
            }
        });

        let results = join_all(requests).await;

        for (result, duration) in results {
            response_times.push(duration);
            match result {
                Ok(response) if response.status().is_success() => success_count += 1,
                _ => failure_count += 1,
            }
        }

        // Small delay between chunks to prevent overwhelming
        tokio::time::sleep(Duration::from_millis(100)).await;
    }

    let avg_response_time = response_times.iter().map(|d| d.as_secs_f64()).sum::<f64>() / response_times.len() as f64;

    let min_response_time = response_times.iter().min().unwrap().clone();
    let max_response_time = response_times.iter().max().unwrap().clone();

    StressTestMetrics {
        endpoint: endpoint.to_string(),
        requests: num_requests,
        success_count,
        failure_count,
        avg_response_time,
        min_response_time,
        max_response_time,
    }
}

#[tokio::test]
async fn test_stress_rag_server() {
    let client = Client::builder().timeout(Duration::from_secs(30)).build().unwrap();

    // Test parameters
    let num_requests = 100;
    let concurrent_requests = 10;

    // Test different endpoints
    let test_cases = vec![
        (
            "/ask",
            json!({
                "query": "What is Bioma?",
            }),
        ),
        (
            "/retrieve",
            json!({
                "query": "How to use actors?",
                "limit": 5,
                "threshold": 0.0
            }),
        ),
        (
            "/rerank",
            json!({
                "query": "What is the weather?",
                "texts": [
                    "The weather is sunny today",
                    "It's raining outside",
                    "The temperature is 25 degrees"
                ]
            }),
        ),
        (
            "/api/embed",
            json!({
                "model": "nomic-embed-text",
                "input": ["Hello world", "How are you?"]
            }),
        ),
    ];

    let mut all_metrics = Vec::new();

    // Run tests for each endpoint
    for (endpoint, payload) in test_cases {
        println!("Testing endpoint: {}", endpoint);

        let metrics = stress_test_endpoint(&client, endpoint, payload, num_requests, concurrent_requests).await;

        println!("\nMetrics for {}:", endpoint);
        println!("Total requests: {}", metrics.requests);
        println!("Successful requests: {}", metrics.success_count);
        println!("Failed requests: {}", metrics.failure_count);
        println!("Average response time: {:.3}s", metrics.avg_response_time);
        println!("Min response time: {:?}", metrics.min_response_time);
        println!("Max response time: {:?}", metrics.max_response_time);
        println!("Success rate: {:.1}%", (metrics.success_count as f64 / metrics.requests as f64) * 100.0);

        all_metrics.push(metrics);
    }

    // Verify the stress test results
    for metrics in &all_metrics {
        assert!(
            metrics.success_count as f64 / metrics.requests as f64 >= 0.95,
            "Success rate below 95% for endpoint {}",
            metrics.endpoint
        );

        assert!(
            metrics.avg_response_time < 5.0,
            "Average response time too high for endpoint {}: {:.3}s",
            metrics.endpoint,
            metrics.avg_response_time
        );
    }
}

#[tokio::test]
async fn test_concurrent_indexing_and_querying() {
    let client = Client::builder().timeout(Duration::from_secs(30)).build().unwrap();

    // First index some test data
    let index_payload = json!({
        "globs": ["./test_data/**/*.md"]
    });

    let index_response = client
        .post("http://localhost:8080/index")
        .json(&index_payload)
        .send()
        .await
        .expect("Failed to index test data");

    assert!(index_response.status().is_success());

    // Now run concurrent queries while also performing indexing
    let num_concurrent_operations = 20;

    let operations = (0..num_concurrent_operations).map(|i| {
        let client = client.clone();
        let index_payload = index_payload.clone();

        async move {
            if i % 2 == 0 {
                // Perform a query
                let query_payload = json!({
                    "query": "What is Bioma?",
                });

                client.post("http://localhost:8080/ask").json(&query_payload).send().await
            } else {
                // Perform indexing
                client.post("http://localhost:8080/index").json(&index_payload).send().await
            }
        }
    });

    let results = join_all(operations).await;

    // Verify results
    for result in results {
        assert!(result.is_ok(), "Operation failed: {:?}", result.err());
        assert!(result.unwrap().status().is_success());
    }
}
