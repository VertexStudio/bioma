use crate::error::ActorError;
use surrealdb::engine::any::Any;
use surrealdb::engine::any::IntoEndpoint;
use surrealdb::opt::auth::Root;
use surrealdb::Surreal;

#[macro_export]
macro_rules! dbg_export_db {
    ($engine:expr) => {{
        let workspace_root = std::env::var("CARGO_MANIFEST_DIR")
            .map(std::path::PathBuf::from)
            .ok()
            .and_then(|path| path.parent().map(|p| p.to_path_buf()))
            .unwrap_or_else(|| std::path::PathBuf::from("."));

        let output_dir = workspace_root.join("output").join("db");
        std::fs::create_dir_all(&output_dir)?;

        let file_name = format!("dbg_{}_{}", file!().replace("/", "_").replace(".", "_"), line!());
        let file_path = output_dir.join(format!("{}.surql", file_name));

        $engine.db().export(file_path.to_str().unwrap()).await?
    }};
}

/// The engine is the main entry point for the Actor framework.
/// Responsible for creating and managing the database connection.
#[derive(Clone, Debug)]
pub struct Engine {
    db: Box<Surreal<Any>>,
}

impl Engine {

    pub fn db(&self) -> &Surreal<Any> {
        &self.db
    }

    pub async fn connect(address: impl IntoEndpoint) -> Result<Engine, ActorError> {
        let db: Surreal<Any> = Surreal::init();
        db.connect(address).await?;
        db.signin(Root { username: "root", password: "root" }).await?;
        db.use_ns("N").use_db("D").await?;
        Ok(Engine { db: Box::new(db) })
    }

    pub async fn test() -> Result<Engine, ActorError> {
        let db: Surreal<Any> = Surreal::init();
        db.connect("memory").await?;
        db.use_ns("N").use_db("D").await?;
        Ok(Engine { db: Box::new(db) })
    }

    pub async fn health(&self) -> bool {
        self.db.health().await.is_ok()
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
