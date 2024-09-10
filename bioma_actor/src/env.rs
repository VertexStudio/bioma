use log::*;
use std::path::PathBuf;

#[derive(Clone, Debug)]
pub struct Config {
    pub s3: S3,
}

impl Env for Config {
    fn from_env() -> Self {
        Config {
            s3: S3::from_env(),
        }
    }
}

#[derive(Clone, Debug)]
pub struct S3 {
    pub region: String,
    pub bucket_name: String,
    pub access_key_id: String,
    pub secret_access_key: String,
}

impl Env for S3 {
    fn from_env() -> Self {
        S3 {
            region: env("AWS_REGION"),
            bucket_name: env("S3_BUCKET_NAME"),
            access_key_id: env("AWS_ACCESS_KEY_ID"),
            secret_access_key: env("AWS_SECRET_ACCESS_KEY"),
        }
    }
}

fn env(key: &str) -> String {
    let value = std::env::var(key)
        .unwrap_or_else(|_| panic!("{} environment variable is not set", key));
    info!("{}: {}", key, value);
    value
}

fn _env_option(key: &str) -> Option<String> {
    let value = std::env::var(key).ok();
    info!("{}: {:?}", key, value);
    value
}

fn _require_path(path: &PathBuf) {
    if !path.exists() {
        panic!("{} does not exist", path.display());
    }
}

pub trait Env {
    fn from_env() -> Self;

//     fn init_logging() {
//         let env_logger = env_logger::try_init_from_env(env_logger::Env::default().default_filter_or("info"));
//         if env_logger::try_init().is_ok() {
            // color_backtrace::install();
            // color_backtrace::BacktracePrinter::new()
            //     .message("BOOM! 💥")
            //     .install(color_backtrace::default_output_stream());
//         } else {
//             println!("Logger already initialized");
//         }
//     }
}

#[cfg(test)]
mod tests {
    use super::*;
    use test_log::test;

    #[test]
    fn test_config_from_env() {
        dotenv::dotenv().ok();
        let _ = Config::from_env();
    }
}
