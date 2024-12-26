use crate::schema::{CallToolResult, TextContent, Tool, ToolInputSchema};
use crate::tools::{ToolDef, ToolError};
use readability::ExtractOptions;
use reqwest::header::CONTENT_TYPE;
use robotstxt::DefaultMatcher;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use url::Url;

const FETCH_SCHEMA: &str = r#"{
    "type": "object",
    "properties": {
        "url": {
            "description": "URL to fetch",
            "type": "string"
        },
        "max_length": {
            "description": "Maximum number of characters to return",
            "type": "integer",
            "default": 5000
        },
        "start_index": {
            "description": "Start content from this character index",
            "type": "integer",
            "default": 0
        },
        "raw": {
            "description": "Get raw content without markdown conversion",
            "type": "boolean",
            "default": false
        }
    },
    "required": ["url"]
}"#;

#[derive(Serialize, Deserialize, JsonSchema)]
pub struct FetchProperties {
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
            client: reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(30))
                .build()
                .unwrap_or_default(),
            user_agent: "Bioma/1.0 (+https://github.com/BiomaAI/bioma)".to_string(),
        }
    }
}

impl ToolDef for Fetch {
    const NAME: &'static str = "fetch";
    const DESCRIPTION: &'static str =
        "Fetches a URL from the internet and extracts its contents as markdown";
    type Properties = FetchProperties;

    fn def() -> Tool {
        let input_schema = serde_json::from_str::<ToolInputSchema>(FETCH_SCHEMA).unwrap();
        Tool {
            name: Self::NAME.to_string(),
            description: Some(Self::DESCRIPTION.to_string()),
            input_schema,
        }
    }

    async fn call(&self, properties: Self::Properties) -> Result<CallToolResult, ToolError> {
        // Validate URL
        let url = Url::parse(&properties.url);
        let url = match url {
            Ok(url) => url,
            Err(e) => return Ok(Self::error(format!("Invalid URL: {}", e))),
        };

        // Check robots.txt
        if let Err(e) = self.check_robots_txt(&url).await {
            return Ok(Self::error(format!("Access denied by robots.txt: {}", e)));
        }

        // Fetch the webpage
        let response = match self.fetch_url(&url).await {
            Ok(r) => r,
            Err(e) => return Ok(Self::error(format!("Failed to fetch URL: {}", e))),
        };

        // Process content
        let content = self.process_content(&url, response, &properties).await;
        let content = match content {
            Ok(content) => content,
            Err(e) => return Ok(Self::error(format!("Failed to process content: {}", e))),
        };

        // Create result
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

    async fn check_robots_txt(&self, url: &Url) -> Result<(), ToolError> {
        let robots_url = url
            .join("/robots.txt")
            .map_err(|e| ToolError::Custom(format!("Failed to construct robots.txt URL: {}", e)))?;

        let response = self
            .client
            .get(robots_url)
            .header("User-Agent", &self.user_agent)
            .send()
            .await;

        match response {
            Ok(resp) => {
                if resp.status().is_client_error() {
                    return Ok(()); // No robots.txt, assume allowed
                }

                let robots_content = resp
                    .text()
                    .await
                    .map_err(|e| ToolError::Custom(format!("Failed to read robots.txt: {}", e)))?;

                let mut matcher = DefaultMatcher::default();
                if !matcher.one_agent_allowed_by_robots(
                    &robots_content,
                    &self.user_agent,
                    url.as_str(),
                ) {
                    return Err(ToolError::Custom("Access denied by robots.txt".to_string()));
                }
                Ok(())
            }
            Err(_) => Ok(()), // Failed to fetch robots.txt, assume allowed
        }
    }

    async fn fetch_url(&self, url: &Url) -> Result<reqwest::Response, reqwest::Error> {
        self.client
            .get(url.as_str())
            .header("User-Agent", &self.user_agent)
            .send()
            .await
    }

    async fn process_content(
        &self,
        url: &Url,
        response: reqwest::Response,
        properties: &FetchProperties,
    ) -> Result<String, ToolError> {
        let content_type = response
            .headers()
            .get(CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .unwrap_or_default()
            .to_string();

        let html = response
            .text()
            .await
            .map_err(|e| ToolError::Custom(format!("Failed to get response text: {}", e)))?;

        let is_html = html.trim().starts_with("<html") || content_type.contains("text/html");

        let content = if properties.raw.unwrap_or(false) || !is_html {
            html
        } else {
            // Convert the HTML string into a cursor that implements Read
            let mut cursor = std::io::Cursor::new(html);

            // Use readability for main content extraction
            let readable = readability::extract(&mut cursor, url, ExtractOptions::default());
            let readable = match readable {
                Ok(readable) => readable,
                Err(e) => {
                    return Err(ToolError::Custom(format!(
                        "Failed to extract content: {}",
                        e
                    )))
                }
            };

            // Convert to markdown
            html2md::parse_html(&readable.content)
        };

        // Apply start_index and max_length
        let start = properties.start_index.unwrap_or(0);
        let content = if start < content.len() {
            content[start..].to_string()
        } else {
            String::new()
        };

        let content = if let Some(max_length) = properties.max_length {
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

    #[tokio::test]
    async fn test_fetch_with_robots_txt() {
        // Create async server
        let mut server = mockito::Server::new_async().await;

        // Mock robots.txt
        let robots_mock = server
            .mock("GET", "/robots.txt")
            .with_status(200)
            .with_header("content-type", "text/plain")
            .with_body("User-agent: *\nDisallow: /private/")
            .create_async()
            .await;

        // Mock HTML page
        let html_mock = server
            .mock("GET", "/test")
            .with_status(200)
            .with_header("content-type", "text/html")
            .with_body("<html><body><h1>Test Page</h1><p>Content</p></body></html>")
            .create_async()
            .await;

        let tool = Fetch::default();

        // Test allowed URL
        let props = FetchProperties {
            url: format!("{}/test", server.url()),
            max_length: None,
            start_index: None,
            raw: None,
        };

        let result = tool.call(props).await.unwrap();
        assert_eq!(result.is_error, Some(false));

        // Test disallowed URL
        let props = FetchProperties {
            url: format!("{}/private/test", server.url()),
            max_length: None,
            start_index: None,
            raw: None,
        };

        let result = tool.call(props).await.unwrap();
        assert_eq!(result.is_error, Some(true));

        // Clean up mocks
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
        let props = FetchProperties {
            url: format!("{}/raw", server.url()),
            max_length: None,
            start_index: None,
            raw: Some(true),
        };

        let result = tool.call(props).await.unwrap();
        assert_eq!(result.is_error, Some(false));
        assert!(result.content[0]
            .get("text")
            .unwrap()
            .as_str()
            .unwrap()
            .contains("<html><body>"));

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

        // Test max_length
        let props = FetchProperties {
            url: format!("{}/limited", server.url()),
            max_length: Some(5),
            start_index: None,
            raw: Some(true),
        };

        let result = tool.call(props).await.unwrap();
        assert_eq!(
            result.content[0].get("text").unwrap().as_str().unwrap(),
            "12345"
        );

        // Test start_index
        let props = FetchProperties {
            url: format!("{}/limited", server.url()),
            max_length: None,
            start_index: Some(5),
            raw: Some(true),
        };

        let result = tool.call(props).await.unwrap();
        assert_eq!(
            result.content[0].get("text").unwrap().as_str().unwrap(),
            "67890"
        );

        html_mock.remove_async().await;
    }

    #[tokio::test]
    async fn test_fetch_error_cases() {
        let mut server = mockito::Server::new_async().await;

        // Test 404 response
        let not_found_mock = server
            .mock("GET", "/not-found")
            .with_status(404)
            .create_async()
            .await;

        let tool = Fetch::default();
        let props = FetchProperties {
            url: format!("{}/not-found", server.url()),
            max_length: None,
            start_index: None,
            raw: None,
        };

        let result = tool.call(props).await.unwrap();
        assert_eq!(result.is_error, Some(true));

        // Test invalid URL
        let props = FetchProperties {
            url: "not-a-url".to_string(),
            max_length: None,
            start_index: None,
            raw: None,
        };

        let result = tool.call(props).await.unwrap();
        assert_eq!(result.is_error, Some(true));

        not_found_mock.remove_async().await;
    }
}
