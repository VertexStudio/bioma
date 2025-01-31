use bioma_actor::prelude::*;
use bioma_llm::embeddings::EmbeddingsError;
use bioma_llm::indexer::{
    CodeLanguage, ContentSource, ImageDimensions, ImageMetadata, Metadata, TextMetadata, TextType,
};
use bioma_llm::prelude::*;
use bioma_llm::retriever::{Context, RetrievedContext};
use test_log::test;
use tracing::error;

#[derive(thiserror::Error, Debug)]
enum TestError {
    #[error("System error: {0}")]
    System(#[from] SystemActorError),
    #[error("Retriever error: {0}")]
    Retriever(#[from] RetrieverError),
    #[error("Embeddings error: {0}")]
    Embeddings(#[from] EmbeddingsError),
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
}

#[test(tokio::test)]
async fn test_retrieved_context_formatting() -> Result<(), TestError> {
    // Create test data
    let context = RetrievedContext {
        context: vec![
            Context {
                text: Some("This is text content".to_string()),
                source: Some(ContentSource {
                    source: "source1.txt".to_string(),
                    uri: "file://source1.txt".to_string(),
                }),
                metadata: Some(Metadata::Text(TextMetadata {
                    content: TextType::Code(CodeLanguage::Rust),
                    chunk_number: 1,
                })),
            },
            Context {
                text: None,
                source: Some(ContentSource { source: "image1.jpg".to_string(), uri: "file://image1.jpg".to_string() }),
                metadata: Some(Metadata::Image(ImageMetadata {
                    format: "jpeg".to_string(),
                    dimensions: ImageDimensions { width: 1920, height: 1080 },
                    size_bytes: 1024000,
                    modified: 1234567890,
                    created: 1234567800,
                })),
            },
        ],
    };

    // Test to_markdown format
    let markdown = context.to_markdown();
    let sections: Vec<&str> = markdown.split("---").map(|s| s.trim()).filter(|s| !s.is_empty()).collect();

    assert_eq!(sections.len(), 2, "Should have 2 sections in markdown");

    // Verify text section
    let text_section = sections[0];
    assert!(text_section.contains("[URI:file://source1.txt]"), "Missing source URI in text section");
    assert!(text_section.contains("[CHUNK:1]"), "Missing chunk number in text section");
    assert!(text_section.contains("This is text content"), "Missing text content");

    // Verify image section
    let image_section = sections[1];
    assert!(image_section.contains("[URI:file://image1.jpg]"), "Missing image URI");
    assert!(image_section.contains("[IMAGE:jpeg]"), "Missing image format tag");

    // Test to_json format
    let json_output = context.to_json();
    let parsed_json: serde_json::Value = serde_json::from_str(&json_output).expect("Failed to parse generated JSON");

    let expected_json = serde_json::json!({
        "context": [
            {
                "text": "This is text content",
                "source": {
                    "source": "source1.txt",
                    "uri": "file://source1.txt"
                },
                "metadata": {
                    "Text": {
                        "content": { "Code": "Rust" },
                        "chunk_number": 1
                    }
                }
            },
            {
                "text": null,
                "source": {
                    "source": "image1.jpg",
                    "uri": "file://image1.jpg"
                },
                "metadata": {
                    "Image": {
                        "format": "jpeg",
                        "dimensions": {
                            "width": 1920,
                            "height": 1080
                        },
                        "size_bytes": 1024000,
                        "modified": 1234567890,
                        "created": 1234567800
                    }
                }
            }
        ]
    });

    assert_eq!(parsed_json, expected_json, "JSON structure mismatch");

    // Test empty context
    let empty_context = RetrievedContext { context: vec![] };
    let empty_json = empty_context.to_json();
    let parsed_empty: serde_json::Value =
        serde_json::from_str(&empty_json).expect("Failed to parse empty context JSON");

    assert_eq!(parsed_empty, serde_json::json!({ "context": [] }), "Empty context should have empty array");

    Ok(())
}
