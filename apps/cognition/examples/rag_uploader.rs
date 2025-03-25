use anyhow::{Context, Result};
use clap::Parser;
use glob::glob;
use reqwest::Client;
use serde::Serialize;
use serde_json::json;
use std::fs;
use std::path::PathBuf;

#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Args {
    /// Bucket name
    #[arg(short, long, default_value = "bioma")]
    bucket: String,

    /// Glob pattern for files to upload
    #[arg(short, long, default_value = "./bioma_*/**/*.rs")]
    upload: String,

    /// Server URL
    #[arg(short, long, default_value = "http://localhost:5766")]
    endpoint: String,
}

#[derive(Debug, Serialize)]
struct Metadata {
    bucket: Option<String>,
    path: std::path::PathBuf,
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();
    let client = Client::new();

    // Upload files
    for entry in glob(&args.upload).context("Failed to read glob pattern")? {
        let path = entry.context("Failed to get glob entry")?;
        upload_file(&client, &args.endpoint, &args.bucket, &path).await?;
    }

    // Index the uploaded files
    index_files(&client, &args.endpoint, &args.upload).await?;

    println!("Indexing completed");
    Ok(())
}

async fn upload_file(client: &Client, server_url: &str, bucket: &str, path: &PathBuf) -> Result<()> {
    println!("Uploading: {}", path.display());

    let file_content = fs::read(path).context("Failed to read file")?;
    let file_name = path.file_name().unwrap().to_str().unwrap();

    let metadata = Metadata { bucket: Some(bucket.to_string()), path: path.to_path_buf() };

    let form = reqwest::multipart::Form::new()
        .part("file", reqwest::multipart::Part::bytes(file_content).file_name(file_name.to_string()))
        .part(
            "metadata",
            reqwest::multipart::Part::text(serde_json::to_string(&metadata).unwrap()).mime_str("application/json")?,
        );

    let response = client
        .post(format!("{}/upload", server_url))
        .multipart(form)
        .send()
        .await
        .context("Failed to send upload request")?;

    if response.status().is_success() {
        println!("Uploaded: {}", path.display());
    } else {
        println!("Failed to upload: {}", path.display());
    }

    Ok(())
}

async fn index_files(client: &Client, server_url: &str, upload_glob: &str) -> Result<()> {
    let response = client
        .post(format!("{}/index", server_url))
        .json(&json!({
            "globs": [upload_glob]
        }))
        .send()
        .await
        .context("Failed to send index request")?;

    if response.status().is_success() {
        println!("Indexing request sent successfully");
    } else {
        println!("Failed to send indexing request");
    }

    Ok(())
}
