use std::borrow::Cow;

use bioma_actor::{
    Actor, ActorContext, ActorError, ActorId, Engine, EngineOptions, Message, SendOptions, SpawnExistsOptions,
    SpawnOptions, SystemActorError,
};
use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use tracing::{error, info};

// Custom error types
#[derive(Debug, thiserror::Error)]
enum StoryError {
    #[error("System error: {0}")]
    System(#[from] SystemActorError),
    #[error("Story generation failed: {0}")]
    GenerationFailed(String),
}

impl ActorError for StoryError {}

// Message types
#[derive(Clone, Debug, Serialize, Deserialize)]
struct GenerateElements {
    theme: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct StoryElement {
    element_type: String,
    description: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct WeaveStory {
    elements: Vec<StoryElement>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct StoryPart {
    content: String,
    part_number: usize,
}

// Story Elements Generator Actor
#[derive(Debug, Serialize, Deserialize)]
struct ElementsGeneratorActor {
    elements_per_story: usize,
}

impl Actor for ElementsGeneratorActor {
    type Error = StoryError;

    async fn start(&mut self, ctx: &mut ActorContext<Self>) -> Result<(), Self::Error> {
        info!("{} Story elements generator started!", ctx.id());
        let mut stream = ctx.recv().await?;

        while let Some(Ok(frame)) = stream.next().await {
            if let Some(message) = frame.is::<GenerateElements>() {
                info!("{} Generating elements for theme: {}", ctx.id(), message.theme);
                self.reply(ctx, &message, &frame).await?;
            }
        }

        Ok(())
    }
}

impl Message<GenerateElements> for ElementsGeneratorActor {
    type Response = StoryElement;

    async fn handle(&mut self, ctx: &mut ActorContext<Self>, message: &GenerateElements) -> Result<(), Self::Error> {
        // Validate theme
        if message.theme.is_empty() {
            return Err(StoryError::GenerationFailed("Theme cannot be empty".to_string()));
        }

        // Generate different story elements based on theme
        let elements = match message.theme.as_str() {
            "fantasy" => vec![
                ("character", "a young wizard apprentice with untamed magical powers"),
                ("setting", "an ancient floating castle filled with magical artifacts"),
                ("conflict", "a curse that's slowly turning everyone into crystal statues"),
                ("object", "a sentient spellbook that gives questionable advice"),
            ],
            "scifi" => vec![
                ("character", "a rogue AI developing emotions"),
                ("setting", "a space station at the edge of a quantum anomaly"),
                ("conflict", "mysterious signals causing technology to malfunction"),
                ("object", "an artifact from a long-lost alien civilization"),
            ],
            theme => return Err(StoryError::GenerationFailed(format!("Unsupported theme: {}", theme))),
        };

        // Stream each element with a small delay
        for (element_type, description) in elements.into_iter().take(self.elements_per_story) {
            ctx.reply(StoryElement { element_type: element_type.to_string(), description: description.to_string() })
                .await?;

            // Add a small delay between elements
            tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        }

        Ok(())
    }
}

// Story Weaver Actor
#[derive(Debug, Serialize, Deserialize)]
struct StoryWeaverActor {
    current_story: Vec<StoryElement>,
}

impl Actor for StoryWeaverActor {
    type Error = StoryError;

    async fn start(&mut self, ctx: &mut ActorContext<Self>) -> Result<(), Self::Error> {
        info!("{} Story weaver started!", ctx.id());
        let mut stream = ctx.recv().await?;

        while let Some(Ok(frame)) = stream.next().await {
            if let Some(message) = frame.is::<WeaveStory>() {
                info!("{} Weaving story from {} elements", ctx.id(), message.elements.len());
                self.reply(ctx, &message, &frame).await?;
            }
        }

        Ok(())
    }
}

impl Message<WeaveStory> for StoryWeaverActor {
    type Response = StoryPart;

    async fn handle(&mut self, ctx: &mut ActorContext<Self>, message: &WeaveStory) -> Result<(), Self::Error> {
        let elements = &message.elements;

        // Extract story elements
        let default_character = "someone".to_string();
        let character = elements
            .iter()
            .find(|e| e.element_type == "character")
            .map(|e| &e.description)
            .unwrap_or(&default_character);

        let default_setting = "somewhere".to_string();
        let setting =
            elements.iter().find(|e| e.element_type == "setting").map(|e| &e.description).unwrap_or(&default_setting);

        let default_conflict = "something happened".to_string();
        let conflict =
            elements.iter().find(|e| e.element_type == "conflict").map(|e| &e.description).unwrap_or(&default_conflict);

        let default_object = "an important item".to_string();
        let object =
            elements.iter().find(|e| e.element_type == "object").map(|e| &e.description).unwrap_or(&default_object);

        // Stream the story in parts
        let parts = vec![
            format!("Our story begins with {} in {}.", character, setting),
            format!("One day, they discovered {}, which would change everything.", object),
            format!("But then, {}!", conflict),
            "And so, the adventure began...".to_string(),
        ];

        for (idx, content) in parts.into_iter().enumerate() {
            ctx.reply(StoryPart { content, part_number: idx + 1 }).await?;

            // Add dramatic pause between parts
            tokio::time::sleep(std::time::Duration::from_secs(1)).await;
        }

        Ok(())
    }
}

// Main coordinator actor
#[derive(Debug, Serialize, Deserialize)]
struct StoryCoordinatorActor {
    generator_id: ActorId,
    weaver_id: ActorId,
    themes: Vec<String>,
}

impl Actor for StoryCoordinatorActor {
    type Error = StoryError;

    async fn start(&mut self, ctx: &mut ActorContext<Self>) -> Result<(), Self::Error> {
        info!("{} Story coordinator started!", ctx.id());

        for theme in &self.themes {
            info!("{} Starting new story with theme: {}", ctx.id(), theme);

            // Collect story elements
            let mut elements = Vec::new();
            let mut element_stream = ctx
                .send::<ElementsGeneratorActor, GenerateElements>(
                    GenerateElements { theme: theme.clone() },
                    &self.generator_id,
                    SendOptions::default(),
                )
                .await?;

            while let Some(element) = element_stream.next().await {
                let element = element?;
                info!("Received element: {} - {}", element.element_type, element.description);
                elements.push(element);
            }

            // Weave the story
            let mut story_stream = ctx
                .send::<StoryWeaverActor, WeaveStory>(WeaveStory { elements }, &self.weaver_id, SendOptions::default())
                .await?;

            while let Some(part) = story_stream.next().await {
                let part = part?;
                info!("Story Part {}: {}", part.part_number, part.content);
            }

            info!("Story complete!\n");
            tokio::time::sleep(std::time::Duration::from_secs(2)).await;
        }

        Ok(())
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Initialize tracing
    let filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info"));
    tracing_subscriber::fmt().with_env_filter(filter).init();

    let options = EngineOptions::builder().endpoint(Cow::Borrowed("ws://localhost:9123")).build();

    // Initialize the actor system
    let engine = Engine::connect(options).await?;

    // Create actor IDs
    let generator_id = ActorId::of::<ElementsGeneratorActor>("/generator");
    let weaver_id = ActorId::of::<StoryWeaverActor>("/weaver");
    let coordinator_id = ActorId::of::<StoryCoordinatorActor>("/coordinator");

    // Spawn the actors
    let (mut generator_ctx, mut generator_actor) = Actor::spawn(
        engine.clone(),
        generator_id.clone(),
        ElementsGeneratorActor { elements_per_story: 4 },
        SpawnOptions::builder().exists(SpawnExistsOptions::Reset).build(),
    )
    .await?;

    let (mut weaver_ctx, mut weaver_actor) = Actor::spawn(
        engine.clone(),
        weaver_id.clone(),
        StoryWeaverActor { current_story: Vec::new() },
        SpawnOptions::builder().exists(SpawnExistsOptions::Reset).build(),
    )
    .await?;

    let (mut coordinator_ctx, mut coordinator_actor) = Actor::spawn(
        engine.clone(),
        coordinator_id.clone(),
        StoryCoordinatorActor {
            generator_id: generator_id.clone(),
            weaver_id: weaver_id.clone(),
            themes: vec!["fantasy".to_string(), "scifi".to_string()],
        },
        SpawnOptions::builder().exists(SpawnExistsOptions::Reset).build(),
    )
    .await?;

    // Start the actors
    let generator_handle = tokio::spawn(async move {
        if let Err(e) = generator_actor.start(&mut generator_ctx).await {
            error!("Generator actor error: {}", e);
        }
    });

    let weaver_handle = tokio::spawn(async move {
        if let Err(e) = weaver_actor.start(&mut weaver_ctx).await {
            error!("Weaver actor error: {}", e);
        }
    });

    // Start the coordinator and wait for it to finish
    coordinator_actor.start(&mut coordinator_ctx).await?;

    // Clean up
    generator_handle.abort();
    weaver_handle.abort();

    Ok(())
}
