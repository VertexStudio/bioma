use crate::roots::{RootsDef, RootsError};
use crate::schema::{ListRootsRequest, ListRootsResult, Root};
use serde::Serialize;

#[derive(Clone, Debug, Serialize)]
pub struct Readme;

impl RootsDef for Readme {
    async fn list_roots(&self, _request: ListRootsRequest) -> Result<ListRootsResult, RootsError> {
        let readme_root = Root { name: Some("readme".to_string()), uri: "file:///bioma/README.md".to_string() };

        Ok(ListRootsResult { roots: vec![readme_root], meta: None })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_readme_roots() {
        let readme = Readme;
        let request = ListRootsRequest { method: "roots/list".to_string(), params: None };

        let result = readme.list_roots(request).await.unwrap();

        assert_eq!(result.roots.len(), 1);
        let root = &result.roots[0];
        assert_eq!(root.name, Some("readme".to_string()));
        assert_eq!(root.uri, "file:///bioma/README.md");
    }
}
