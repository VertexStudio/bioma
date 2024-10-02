use crate::actor::SystemActorError;
use crate::util::find_project_root;
use derive_more::Display;
use object_store::local::LocalFileSystem;
use serde::{Deserialize, Serialize};
use std::borrow::Cow;
use std::path::PathBuf;
use std::time::Duration;
use surrealdb::{
    engine::any::{Any, IntoEndpoint},
    opt::auth::Root,
    value::RecordId,
    Surreal,
};
use tokio::time::sleep;
use tracing::{info, warn};

#[macro_export]
macro_rules! dbg_export_db {
    ($engine:expr) => {{
        let output_dir = $engine.output_dir().join("debug");
        std::fs::create_dir_all(&output_dir).unwrap();
        let file_name = format!("dbg_{}_{}", file!().replace("/", "_").replace(".", "_"), line!());
        let file_path = output_dir.join(format!("{}.surql", file_name));
        $engine.db().export(file_path.to_str().unwrap()).await.unwrap();
    }};
}

// Record struct for database entries
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Display)]
pub struct Record {
    pub id: RecordId,
}

/// Configuration options for the Engine.
#[derive(Clone, Debug, Serialize, Deserialize, bon::Builder)]
pub struct EngineOptions {
    /// Database endpoint
    #[builder(default = "memory".into())]
    pub endpoint: Cow<'static, str>,
    /// The namespace to use in the database.
    #[builder(default = "dev".into())]
    pub namespace: Cow<'static, str>,
    /// The name of the database to connect to.
    #[builder(default = "bioma".into())]
    pub database: Cow<'static, str>,
    /// The username for database authentication.
    #[builder(default = "root".into())]
    pub username: Cow<'static, str>,
    /// The password for database authentication.
    #[builder(default = "root".into())]
    pub password: Cow<'static, str>,
    /// Output directory for artifacts.
    #[builder(default = default_output_dir())]
    pub output_dir: PathBuf,
    /// The local file system path for object store.
    #[builder(default = default_local_store_dir())]
    pub local_store_dir: PathBuf,
    /// The HuggingFace cache directory.
    #[builder(default = default_hf_cache_dir())]
    pub hf_cache_dir: PathBuf,
}

fn default_output_dir() -> PathBuf {
    let project_root = find_project_root();
    project_root.join(".output")
}

fn default_local_store_dir() -> PathBuf {
    let output_dir = default_output_dir();
    output_dir.join("store")
}

fn default_hf_cache_dir() -> PathBuf {
    let output_dir = default_output_dir();
    output_dir.join("cache").join("huggingface").join("hub")
}

impl Default for EngineOptions {
    fn default() -> Self {
        EngineOptions::builder().build()
    }
}

impl EngineOptions {
    pub fn info(&self) {
        info!("Engine: {}, ns: {}, db: {}, user: {}", self.endpoint, self.namespace, self.database, self.username);
    }
}

/// The engine is the main entry point for the Actor framework.
/// Responsible for creating and managing the database connection.
#[derive(Clone, Debug)]
pub struct Engine {
    db: Box<Surreal<Any>>,
    options: EngineOptions,
}

impl Engine {
    pub fn db(&self) -> &Surreal<Any> {
        &self.db
    }

    pub async fn connect(options: EngineOptions) -> Result<Engine, SystemActorError> {
        options.info();

        let mut retry_delay = Duration::from_secs(1);
        let max_delay = Duration::from_secs(10);

        loop {
            match Self::attempt_connect(options.endpoint.to_string(), &options).await {
                Ok(engine) => return Ok(engine),
                Err(e) => {
                    warn!("Failed to connect: {}. Retrying in {:?}...", e, retry_delay);
                    sleep(retry_delay).await;
                    retry_delay = std::cmp::min(retry_delay * 2, max_delay);
                }
            }
        }
    }

    async fn attempt_connect(address: impl IntoEndpoint, options: &EngineOptions) -> Result<Engine, SystemActorError> {
        let db: Surreal<Any> = Surreal::init();
        db.connect(address).await?;
        db.signin(Root { username: &options.username, password: &options.password }).await?;
        db.use_ns(options.namespace.clone()).use_db(options.database.clone()).await?;
        Engine::define(&db).await?;
        Ok(Engine { db: Box::new(db), options: options.clone() })
    }

    pub async fn test() -> Result<Engine, SystemActorError> {
        let options = EngineOptions::default();
        options.info();
        let db: Surreal<Any> = Surreal::init();
        db.connect("memory").await?;
        db.use_ns(options.namespace.clone()).use_db(options.database.clone()).await?;
        Engine::define(&db).await?;
        Ok(Engine { db: Box::new(db), options })
    }

    pub async fn reset(&self) -> Result<(), SystemActorError> {
        let db = self.db.clone();
        let db_name = self.options.database.clone();
        db.use_db("test").await?;
        db.query(format!("REMOVE DATABASE `{}`;", db_name)).await?;
        db.use_ns(self.options.namespace.clone()).use_db(db_name.clone()).await?;
        Engine::define(&db).await?;
        Ok(())
    }

    pub async fn health(&self) -> bool {
        self.db.health().await.is_ok()
    }

    async fn define(db: &Surreal<Any>) -> Result<(), SystemActorError> {
        // load the surreal definition file
        // let def = std::fs::read_to_string("assets/surreal/def.surql").unwrap();
        let def = include_str!("../sql/def.surql").parse::<String>().unwrap();
        db.query(&def).await?;
        Ok(())
    }

    pub fn local_store(&self) -> Result<LocalFileSystem, SystemActorError> {
        let store = LocalFileSystem::new_with_prefix(self.options.local_store_dir.clone())?;
        Ok(store)
    }

    pub fn local_store_dir(&self) -> &PathBuf {
        &self.options.local_store_dir
    }

    pub fn output_dir(&self) -> &PathBuf {
        &self.options.output_dir
    }

    pub fn huggingface_cache_dir(&self) -> &PathBuf {
        &self.options.hf_cache_dir
    }

    pub fn options(&self) -> &EngineOptions {
        &self.options
    }

    pub fn info(&self) {
        info!("ns: {}, db: {}, user: {}", self.options.namespace, self.options.database, self.options.username);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_connect() {
        let engine = Engine::test().await.unwrap();
        assert_eq!(engine.health().await, true);
    }
}
