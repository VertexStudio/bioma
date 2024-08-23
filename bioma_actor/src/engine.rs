use crate::error::ActorError;
use once_cell::sync::Lazy;
use surrealdb::engine::any::Any;
use surrealdb::engine::any::IntoEndpoint;
use surrealdb::opt::auth::Root;
use surrealdb::Surreal;

pub static DB: Lazy<Surreal<Any>> = Lazy::new(Surreal::init);

#[macro_export]
macro_rules! dbg_export_db {
    () => {{
        let workspace_root = std::env::var("CARGO_MANIFEST_DIR")
            .map(std::path::PathBuf::from)
            .ok()
            .and_then(|path| path.parent().map(|p| p.to_path_buf()))
            .unwrap_or_else(|| std::path::PathBuf::from("."));

        let output_dir = workspace_root.join("output").join("db");
        std::fs::create_dir_all(&output_dir)?;

        let file_name = format!(
            "dbg_{}_{}",
            file!().replace("/", "_").replace(".", "_"),
            line!()
        );
        let file_path = output_dir.join(format!("{}.surql", file_name));

        DB.export(file_path.to_str().unwrap()).await?
    }};
}

/// The engine is the main entry point for the Actor framework.
/// Responsible for creating and managing the database connection.
#[derive(Clone)]
pub struct Engine {}

impl Engine {
    pub async fn connect(address: impl IntoEndpoint) -> Result<Self, ActorError> {
        DB.connect(address).await?;
        DB.signin(Root {
            username: "root",
            password: "root",
        })
        .await?;
        DB.use_ns("N").use_db("D").await?;
        Ok(Engine {})
    }

    pub async fn test() -> Result<Self, ActorError> {
        DB.connect("memory").await?;
        DB.use_ns("N").use_db("D").await?;
        Ok(Engine {})
    }

    pub async fn health(&self) -> bool {
        DB.health().await.is_ok()
    }

    pub async fn dbg_export_db(&self) -> Result<(), ActorError> {
        dbg_export_db!();
        Ok(())
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
