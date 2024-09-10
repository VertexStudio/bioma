use futures::StreamExt;
use bioma_actor::prelude::*;
use object_store::PutPayload;
use object_store::local::LocalFileSystem;
use object_store::{path::Path, ObjectStore};
use rand::{distributions::Alphanumeric, Rng};
use serde::{Deserialize, Serialize};
use tracing::{error, info};

fn generate_random_bytes(size: usize) -> Vec<u8> {
    rand::thread_rng().sample_iter(&Alphanumeric).take(size).collect()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct S3Uploader {
    bucket: String,
    region: String,
    access_key: String,
    secret_key: String,
}

impl Actor for S3Uploader {
    type Error = SystemActorError;

    async fn start(&mut self, ctx: &mut ActorContext<Self>) -> Result<(), SystemActorError> {
        info!("{} S3Uploader Started", ctx.id());

        let mut stream = ctx.recv().await?;
        while let Some(Ok(frame)) = stream.next().await {
            if let Some(object_upload) = frame.is::<UploadToS3>() {
                self.reply(ctx, &object_upload, &frame).await?;
            }
        }

        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct UploadToS3 {
    path: std::path::PathBuf,
    data: Vec<u8>,
}


impl Message<UploadToS3> for S3Uploader {
    type Response = ();
    
    async fn handle(&mut self, ctx: &mut ActorContext<Self>, msg: &UploadToS3) -> Result<(), SystemActorError> {
        info!("{} Start Upload to S3", ctx.id());

        let s3 = object_store::aws::AmazonS3Builder::new()
            .with_region(self.region.clone())
            .with_bucket_name(self.bucket.clone())
            .with_access_key_id(self.access_key.clone())
            .with_secret_access_key(self.secret_key.clone())
            .build()?;

        let path = Path::parse(&msg.path.to_string_lossy()).map_err(SystemActorError::PathError)?;
        let result = s3
            .put(&path, PutPayload::from_bytes(msg.data.clone().into()))
            .await;

        match result {
            Ok(_) => {
                info!("Successfully uploaded to S3: {:?}", msg.path);
            }
            Err(e) => {
                error!("Error uploading to S3: {}", e);
            }
        }

        info!("{} Finished uploading to S3 {:?} ", ctx.id(), msg.path);

        Ok(())
    }
}


#[derive(Debug, Clone, Serialize, Deserialize)]
struct ObjectSaved {
    path: std::path::PathBuf,
}

#[derive(Debug, Serialize, Deserialize)]
struct RandomObjectSaver {
    prefix: std::path::PathBuf,
    num_objects: usize,
    loader_id: ActorId,
    s3_uploader_id: ActorId
}

impl Actor for RandomObjectSaver {
    type Error = SystemActorError;

    async fn start(&mut self, ctx: &mut ActorContext<Self>) -> Result<(), SystemActorError> {
        info!("{} Started", ctx.id());
        let store = ctx.engine().local_store()?;

        // Generate random objects of 1 to 3 MBs
        for i in 0..self.num_objects {
            let data = generate_random_bytes(1 + rand::thread_rng().gen_range(0..3) * 1_000_000);

            let path = self.prefix.join(i.to_string());
            let path = Path::from_iter(path.iter().map(|c| c.to_string_lossy().to_string()));
            store.put(&path, data.clone().into()).await?;

            // Send to S3 actor
            ctx.do_send::<S3Uploader, UploadToS3>(
                UploadToS3 { path: path.to_string().into(), data: data.clone() },
                &self.s3_uploader_id,
            ).await?;

            // Send to Local Store actor
            ctx.do_send::<RandomObjectLoader, ObjectSaved>(
                ObjectSaved { path: path.to_string().into() },
                &self.loader_id,
            )
            .await?;
        }

        info!("{} Saved {} objects", ctx.id(), self.num_objects);
        Ok(())
    }
}

#[derive(thiserror::Error, Debug)]
pub enum RandomObjectLoaderError {
    #[error("System error: {0}")]
    System(#[from] SystemActorError),
    #[error("LocalFileSystem not initialized")]
    LocalFileSystemNotInitialized,
}

impl ActorError for RandomObjectLoaderError {}

#[derive(Debug, Serialize, Deserialize)]
struct RandomObjectLoader {
    prefix: std::path::PathBuf,
    num_objects: usize,
    #[serde(skip)]
    store: Option<LocalFileSystem>,
}

impl Message<ObjectSaved> for RandomObjectLoader {
    type Response = ();

    async fn handle(&mut self, ctx: &mut ActorContext<Self>, msg: &ObjectSaved) -> Result<(), RandomObjectLoaderError> {
        let Some(store) = &self.store else {
            return Err(RandomObjectLoaderError::LocalFileSystemNotInitialized);
        };
        self.num_objects -= 1;
        info!("{} Received: {:?}", ctx.id(), msg);
        let path = Path::parse(&msg.path.to_string_lossy()).map_err(SystemActorError::PathError)?;
        let data = store.get(&path).await.map_err(SystemActorError::ObjectStore)?;

        info!("{} Data: {:?}", ctx.id(), data);
        Ok(())
    }
}


impl Actor for RandomObjectLoader {
    type Error = RandomObjectLoaderError;

    async fn start(&mut self, ctx: &mut ActorContext<Self>) -> Result<(), RandomObjectLoaderError> {
        self.store = Some(ctx.engine().local_store()?);

        let mut stream = ctx.recv().await?;
        while let Some(Ok(frame)) = stream.next().await {
            if let Some(object_saved) = frame.is::<ObjectSaved>() {
                self.reply(ctx, &object_saved, &frame).await?;


                if self.num_objects == 0 {
                    break;
                }
            }
        }

        Ok(())
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    dotenv::dotenv().ok();
    let env = Config::from_env();

    // Initialize tracing
    let filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info"));
    tracing_subscriber::fmt().with_env_filter(filter).init();

    // Initialize the actor system
    let engine = Engine::test().await?;

    let num_objects = 10;
    let prefix = std::path::PathBuf::from("random_objects");

    // Spawn the RandomObjectLoader actor
    let random_object_loader_id = ActorId::of::<RandomObjectLoader>("/random_object_loader");
    let mut random_object_loader_actor = Actor::spawn(
        &engine,
        &random_object_loader_id,
        RandomObjectLoader { prefix: prefix.clone(), num_objects, store: None },
        SpawnOptions::default(),
    )
    .await?;

    let random_object_loader_handle = tokio::spawn(async move {
        if let Err(e) = random_object_loader_actor.start().await {
            error!("RandomObjectLoader actor error: {}", e);
        }
    });

    tokio::time::sleep(std::time::Duration::from_secs(1)).await;


    // Spawn the S3 Uploader actor
    let s3_uploader_id = ActorId::of::<S3Uploader>("/s3_uploader");
    let mut s3_uploader_actor = Actor::spawn(
        &engine,
        &s3_uploader_id,
        S3Uploader { bucket: env.s3.bucket_name, region: env.s3.region, access_key: env.s3.access_key_id, secret_key: env.s3.secret_access_key },
        SpawnOptions::default(),
    )
    .await?;
    let s3_uploader_handle = tokio::spawn(async move {
        if let Err(e) = s3_uploader_actor.start().await {
            error!("S3UPLOADER actor error: {}", e);
        }
    });

    // Spawn the RandomObjectSaver actor
    let random_object_saver_id = ActorId::of::<RandomObjectSaver>("/random_object_saver");
    let mut random_object_saver_actor = Actor::spawn(
        &engine,
        &random_object_saver_id,
        RandomObjectSaver { prefix: prefix.clone(), num_objects, loader_id: random_object_loader_id, s3_uploader_id },
        SpawnOptions::default(),
    )
    .await?;
    let random_object_saver_handle = tokio::spawn(async move {
        if let Err(e) = random_object_saver_actor.start().await {
            error!("RandomObjectSaver actor error: {}", e);
        }
    });

    // Wait for the RandomObjectLoader actor to finish
    let _ = random_object_loader_handle.await;

    // Wait for the S3UPLOADER actor to finish
    let _ = s3_uploader_handle.await;

    // Wait for the RandomObjectSaver actor to finish
    let _ = random_object_saver_handle.await;

    // Export the database for debugging
    dbg_export_db!(engine);

    Ok(())
}
