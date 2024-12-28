use crate::resources::{ResourceDef, ResourceError};
use crate::schema::{ReadResourceResult, Resource};
use serde::Serialize;

#[derive(Clone, Debug, Serialize)]
pub struct Readme;

impl ResourceDef for Readme {
    const NAME: &'static str = "readme";
    const DESCRIPTION: &'static str = "Returns the Bioma README.md content";
    const URI: &'static str = "file:///bioma/README.md";

    fn def() -> Resource {
        Resource {
            name: Self::NAME.to_string(),
            description: Some(Self::DESCRIPTION.to_string()),
            uri: Self::URI.to_string(),
            mime_type: Some("text/markdown".to_string()),
            annotations: None,
        }
    }

    async fn read(&self, _uri: String) -> Result<ReadResourceResult, ResourceError> {
        let readme_content = include_str!("../../../README.md");
        Ok(ReadResourceResult {
            contents: vec![serde_json::json!({
                "uri": Self::URI.to_string(),
                "mimeType": "text/markdown",
                "text": readme_content,
            })],
            meta: None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_readme() {
        let readme = Readme;
        let result = readme.read("".to_string()).await.unwrap();
        let content = &result.contents[0];
        let text = content["text"].as_str().expect("text field should be a string");
        assert!(text.contains("# Bioma"));
    }

    #[test]
    fn test_readme_schema() {
        let resource = Readme::def();
        assert_eq!(resource.name, "readme");
        assert_eq!(resource.description.unwrap(), "Returns the Bioma README.md content");
    }
}
