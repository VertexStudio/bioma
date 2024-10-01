use crate::prelude::*;
use serde::{Deserialize, Serialize};
use std::env;
use std::path::{Path, PathBuf};

/// A relay actor for sending messages to another actor from outside the actor system
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Relay;

impl Actor for Relay {
    type Error = SystemActorError;

    fn start(&mut self, _ctx: &mut ActorContext<Self>) -> impl Future<Output = Result<(), Self::Error>> {
        async move {
            panic!("Relay should not be started");
        }
    }
}

pub fn find_project_root() -> PathBuf {
    let start_dir = env::var("CARGO_MANIFEST_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));
    let mut current_dir = start_dir.as_path();
    loop {
        // Check for indicators of the true project root
        if is_project_root(current_dir) {
            return current_dir.to_path_buf();
        }
        // Move up to the parent directory
        if let Some(parent) = current_dir.parent() {
            current_dir = parent;
        } else {
            // If we can't find the project root, return the current directory
            return PathBuf::from(".");
        }
    }
}

fn is_project_root(dir: &Path) -> bool {
    // Check for a combination of indicators that suggest this is the true project root
    let has_cargo_toml = dir.join("Cargo.toml").exists();
    let has_git = dir.join(".git").exists();
    let has_gitignore = dir.join(".gitignore").exists();
    let has_workspace = has_cargo_toml && is_workspace_root(dir);
    // Consider it the project root if it has a .git directory or
    // if it's a Cargo workspace root
    (has_git || has_gitignore) && has_cargo_toml || has_workspace
}

fn is_workspace_root(dir: &Path) -> bool {
    if let Ok(content) = std::fs::read_to_string(dir.join("Cargo.toml")) {
        // Check if Cargo.toml contains a [workspace] section
        content.contains("[workspace]")
    } else {
        false
    }
}