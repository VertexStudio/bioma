use base64::Engine;
use ollama_rs::{
    generation::{
        chat::{request::ChatMessageRequest, ChatMessage},
        completion::{request::GenerationRequest, GenerationContext},
        images::Image,
    },
    Ollama,
};

#[tokio::test]
async fn test_ollama_completion() {
    let ollama = Ollama::default();

    let model = "llama3.1".to_string();
    let prompt = "Why is the sky blue?".to_string();

    let res = ollama.generate(GenerationRequest::new(model, prompt)).await;

    if let Ok(res) = res {
        println!("{}", res.response);
    }
}

#[test]
fn test_ollama_chat_history_saved_as_should() {
    let mut ollama = Ollama::new_default_with_history(30);
    let chat_id = "default".to_string();

    ollama.add_user_response(chat_id.clone(), "Hello".to_string());
    ollama.add_assistant_response(chat_id.clone(), "Hi".to_string());

    ollama.add_user_response(chat_id.clone(), "Tell me 'hi' again".to_string());
    ollama.add_assistant_response(chat_id.clone(), "Hi again".to_string());

    assert_eq!(
        ollama.get_messages_history(chat_id.clone()).unwrap().len(),
        4
    );

    let last = ollama.get_messages_history(chat_id).unwrap().last();
    assert!(last.is_some());
    assert_eq!(last.unwrap().content, "Hi again".to_string());
}

#[tokio::test]
async fn test_ollama_generation_with_images() {
    let ollama = Ollama::default();

    let bytes = include_bytes!("../../assets/images/elephant.jpg");
    let base64 = base64::engine::general_purpose::STANDARD.encode(&bytes);
    let image = Image::from_base64(&base64);

    let res = ollama
        .generate(
            GenerationRequest::new(
                "llava-llama3".to_string(),
                "What can we see in this image?".to_string(),
            )
            .add_image(image),
        )
        .await
        .unwrap();
    dbg!(res);
}

#[tokio::test]
async fn test_ollama_send_chat_messages_with_images() {
    let ollama = Ollama::default();

    let bytes = include_bytes!("../../assets/images/elephant.jpg");
    let base64 = base64::engine::general_purpose::STANDARD.encode(bytes);
    let image = Image::from_base64(&base64);

    let messages =
        vec![ChatMessage::user("What can we see in this image?".to_string()).add_image(image)];
    let res = ollama
        .send_chat_messages(ChatMessageRequest::new(
            "llava-llama3".to_string(),
            messages,
        ))
        .await
        .unwrap();
    dbg!(&res);

    assert!(res.done);
}

#[tokio::test]
async fn test_ollama_embeddings_generation() {
    let ollama = Ollama::default();

    let prompt = "Why is the sky blue?".to_string();

    let res = ollama
        .generate_embeddings("nomic-embed-text".to_string(), prompt, None)
        .await
        .unwrap();

    dbg!(res);
}