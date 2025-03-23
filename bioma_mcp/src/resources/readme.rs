use crate::resources::{ResourceDef, ResourceError};
use crate::schema::ReadResourceResult;
use serde::Serialize;

#[derive(Clone, Debug, Serialize)]
pub struct Readme;

impl ResourceDef for Readme {
    const NAME: &'static str = "readme";
    const DESCRIPTION: &'static str = "Returns the Bioma README.md content";
    const URI: &'static str = "file:///bioma/README.md";
    const MIME_TYPE: Option<&'static str> = Some("text/markdown");

    async fn read(&self, uri: String) -> Result<ReadResourceResult, ResourceError> {
        if uri != Self::URI {
            return Err(ResourceError::NotFound(format!("Resource not found: {}", uri)));
        }

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
        let result = readme.read(Readme::URI.to_string()).await.unwrap();
        let content = &result.contents[0];
        let text = content["text"].as_str().expect("text field should be a string");
        assert!(text.contains("# Bioma"));
    }

    #[tokio::test]
    async fn test_readme_not_found() {
        let readme = Readme;
        let result = readme.read("file:///nonexistent".to_string()).await;
        assert!(result.is_err());
        match result {
            Err(ResourceError::NotFound(_)) => {}
            _ => panic!("Expected NotFound error"),
        }
    }

    #[test]
    fn test_readme_schema() {
        let resource = Readme::def();
        assert_eq!(resource.name, "readme");
        assert_eq!(resource.description.unwrap(), "Returns the Bioma README.md content");
        assert_eq!(resource.mime_type.unwrap(), "text/markdown");
    }
}
