use crate::schema::{CallToolResult, TextContent};
use crate::server::RequestContext;
use crate::tools::ToolDef;
use anyhow::{anyhow, Error};
use readability::ExtractOptions;
use reqwest::header::CONTENT_TYPE;
use robotstxt::DefaultMatcher;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use url::Url;

#[derive(Serialize, Deserialize, JsonSchema)]
pub struct FetchArgs {
    #[schemars(description = "URL to fetch", required = true)]
    url: String,
    #[schemars(description = "Maximum number of characters to return")]
    max_length: Option<usize>,
    #[schemars(description = "Start content from this character index")]
    start_index: Option<usize>,
    #[schemars(description = "Get raw content without markdown conversion")]
    raw: Option<bool>,
}

#[derive(Clone, Debug, Serialize)]
pub struct Fetch {
    #[serde(skip)]
    client: reqwest::Client,
    user_agent: String,
}

impl Default for Fetch {
    fn default() -> Self {
        Self {
            client: reqwest::Client::builder().timeout(std::time::Duration::from_secs(30)).build().unwrap_or_default(),
            user_agent: "Bioma/1.0 (+https://github.com/BiomaAI/bioma)".to_string(),
        }
    }
}

impl ToolDef for Fetch {
    const NAME: &'static str = "fetch";
    const DESCRIPTION: &'static str = "Fetches a URL from the internet and extracts its contents as markdown";
    type Args = FetchArgs;

    async fn call(&self, args: Self::Args, _request_context: RequestContext) -> Result<CallToolResult, Error> {
        let url = Url::parse(&args.url);
        let url = match url {
            Ok(url) => url,
            Err(e) => return Ok(Self::error(format!("Invalid URL: {}", e))),
        };

        if let Err(e) = self.check_robots_txt(&url).await {
            return Ok(Self::error(format!("Access denied by robots.txt: {}", e)));
        }

        let response = match self.fetch_url(&url).await {
            Ok(r) => r,
            Err(e) => return Ok(Self::error(format!("Failed to fetch URL: {}", e))),
        };

        let content = self.process_content(&url, response, &args).await;
        let content = match content {
            Ok(content) => content,
            Err(e) => return Ok(Self::error(format!("Failed to process content: {}", e))),
        };

        let result = Self::success(&content);

        Ok(result)
    }
}

impl Fetch {
    fn error(error_message: impl Into<String>) -> CallToolResult {
        CallToolResult {
            content: vec![serde_json::to_value(TextContent {
                type_: "text".to_string(),
                text: error_message.into(),
                annotations: None,
            })
            .unwrap()],
            is_error: Some(true),
            meta: None,
        }
    }

    fn success(message: impl Into<String>) -> CallToolResult {
        CallToolResult {
            content: vec![serde_json::to_value(TextContent {
                type_: "text".to_string(),
                text: message.into(),
                annotations: None,
            })
            .unwrap()],
            is_error: Some(false),
            meta: None,
        }
    }

    async fn check_robots_txt(&self, url: &Url) -> Result<(), Error> {
        let robots_url = url.join("/robots.txt").map_err(|e| anyhow!("Failed to construct robots.txt URL: {}", e))?;

        let response = self.client.get(robots_url).header("User-Agent", &self.user_agent).send().await;

        match response {
            Ok(resp) => {
                if resp.status().is_client_error() {
                    return Ok(());
                }

                let robots_content = resp.text().await.map_err(|e| anyhow!("Failed to read robots.txt: {}", e))?;

                let mut matcher = DefaultMatcher::default();
                if !matcher.one_agent_allowed_by_robots(&robots_content, &self.user_agent, url.as_str()) {
                    return Err(anyhow!("Access denied by robots.txt for URL: {}", url));
                }
                Ok(())
            }
            Err(_) => Ok(()),
        }
    }

    async fn fetch_url(&self, url: &Url) -> Result<reqwest::Response, reqwest::Error> {
        self.client.get(url.as_str()).header("User-Agent", &self.user_agent).send().await
    }

    async fn process_content(&self, url: &Url, response: reqwest::Response, args: &FetchArgs) -> Result<String, Error> {
        let content_type =
            response.headers().get(CONTENT_TYPE).and_then(|v| v.to_str().ok()).unwrap_or_default().to_string();

        let html = response.text().await.map_err(|e| anyhow!("Failed to get response text: {}", e))?;

        let is_html = html.trim().starts_with("<html") || content_type.contains("text/html");

        let content = if args.raw.unwrap_or(false) || !is_html {
            html
        } else {
            let mut cursor = std::io::Cursor::new(html);

            let readable = readability::extract(&mut cursor, url, ExtractOptions::default());
            let readable = match readable {
                Ok(readable) => readable,
                Err(e) => return Err(anyhow!("Failed to extract content: {}", e)),
            };

            html2md::parse_html(&readable.content)
        };

        let start = args.start_index.unwrap_or(0);
        let content = if start < content.len() { content[start..].to_string() } else { String::new() };

        let content = if let Some(max_length) = args.max_length {
            content.chars().take(max_length).collect()
        } else {
            content.chars().take(5000).collect()
        };

        Ok(content)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mockito;

    #[test]
    fn test_auto_generated_schema() {
        let tool = Fetch::def();
        let schema_json = serde_json::to_string_pretty(&tool).unwrap();
        println!("Tool Schema:\n{}", schema_json);
    }

    #[tokio::test]
    async fn test_fetch_with_robots_txt() {
        let mut server = mockito::Server::new_async().await;

        let robots_mock = server
            .mock("GET", "/robots.txt")
            .with_status(200)
            .with_header("content-type", "text/plain")
            .with_body("User-agent: *\nDisallow: /private/")
            .create_async()
            .await;

        let html_mock = server
            .mock("GET", "/test")
            .with_status(200)
            .with_header("content-type", "text/html")
            .with_body("<html><body><h1>Test Page</h1><p>Content</p></body></html>")
            .create_async()
            .await;

        let tool = Fetch::default();

        let props = FetchArgs { url: format!("{}/test", server.url()), max_length: None, start_index: None, raw: None };

        let result = tool.call(props, RequestContext::default()).await.unwrap();
        assert_eq!(result.is_error, Some(false));

        let props =
            FetchArgs { url: format!("{}/private/test", server.url()), max_length: None, start_index: None, raw: None };

        let result = tool.call(props, RequestContext::default()).await.unwrap();
        assert_eq!(result.is_error, Some(true));

        robots_mock.remove_async().await;
        html_mock.remove_async().await;
    }

    #[tokio::test]
    async fn test_fetch_raw_content() {
        let mut server = mockito::Server::new_async().await;

        let html_mock = server
            .mock("GET", "/raw")
            .with_status(200)
            .with_header("content-type", "text/html")
            .with_body("<html><body><h1>Test Page</h1><p>Content</p></body></html>")
            .create_async()
            .await;

        let tool = Fetch::default();
        let props =
            FetchArgs { url: format!("{}/raw", server.url()), max_length: None, start_index: None, raw: Some(true) };

        let result = tool.call(props, RequestContext::default()).await.unwrap();
        assert_eq!(result.is_error, Some(false));
        assert!(result.content[0].get("text").unwrap().as_str().unwrap().contains("<html><body>"));

        html_mock.remove_async().await;
    }

    #[tokio::test]
    async fn test_fetch_with_length_limits() {
        let mut server = mockito::Server::new_async().await;

        let html_mock = server
            .mock("GET", "/limited")
            .with_status(200)
            .with_header("content-type", "text/plain")
            .with_body("1234567890")
            .create_async()
            .await;

        let tool = Fetch::default();

        let props = FetchArgs {
            url: format!("{}/limited", server.url()),
            max_length: Some(5),
            start_index: None,
            raw: Some(true),
        };

        let result = tool.call(props, RequestContext::default()).await.unwrap();
        assert_eq!(result.content[0].get("text").unwrap().as_str().unwrap(), "12345");

        let props = FetchArgs {
            url: format!("{}/limited", server.url()),
            max_length: None,
            start_index: Some(5),
            raw: Some(true),
        };

        let result = tool.call(props, RequestContext::default()).await.unwrap();
        assert_eq!(result.content[0].get("text").unwrap().as_str().unwrap(), "67890");

        html_mock.remove_async().await;
    }

    #[tokio::test]
    async fn test_fetch_error_cases() {
        let mut server = mockito::Server::new_async().await;

        let not_found_mock = server.mock("GET", "/not-found").with_status(404).create_async().await;

        let tool = Fetch::default();
        let props =
            FetchArgs { url: format!("{}/not-found", server.url()), max_length: None, start_index: None, raw: None };

        let result = tool.call(props, RequestContext::default()).await.unwrap();
        assert_eq!(result.is_error, Some(true));

        let props = FetchArgs { url: "not-a-url".to_string(), max_length: None, start_index: None, raw: None };

        let result = tool.call(props, RequestContext::default()).await.unwrap();
        assert_eq!(result.is_error, Some(true));

        not_found_mock.remove_async().await;
    }
}
