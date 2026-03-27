use crate::core::Package;
use serde::Deserialize;

#[derive(Debug, Deserialize)]
pub struct GithubSearchResult {
    pub items: Vec<GithubRepo>,
}

#[derive(Debug, Deserialize)]
pub struct GithubRepo {
    pub full_name: String,
    pub description: Option<String>,
    pub html_url: String,
}

/// Parse GitHub search API response
pub fn parse_github_search(output: &str, backend: &str) -> Vec<Package> {
    let result: GithubSearchResult = serde_json::from_str(output).unwrap_or(GithubSearchResult { items: Vec::new() });
    
    result.items
        .into_iter()
        .map(|repo| Package {
            name: repo.full_name,
            version: None,
            backend: backend.to_string(),
            description: repo.description,
            repository: Some(repo.html_url),
            size: None,
        })
        .collect()
}
