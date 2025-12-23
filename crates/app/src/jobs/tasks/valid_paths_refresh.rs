use std::collections::HashSet;

use tracing::{info, warn};

use crate::jobs::JobError;
use crate::state::AppState;

pub async fn run(state: &AppState) -> Result<(), JobError> {
    let url = state.config.valid_paths_url.trim();
    if url.is_empty() {
        warn!("valid paths url not configured; skip refresh");
        return Ok(());
    }
    let response = state.http_client.get(url).send().await?.error_for_status()?;
    let body = response.text().await?;
    let paths = parse_valid_paths(&body);
    let count = paths.len();
    {
        let mut guard = state.valid_paths.write().await;
        *guard = paths;
    }
    info!(count, "valid paths refreshed");
    Ok(())
}

fn parse_valid_paths(input: &str) -> HashSet<String> {
    let mut paths = HashSet::new();
    for line in input.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        if !trimmed.starts_with('/') {
            continue;
        }
        paths.insert(trimmed.to_string());
    }
    paths
}

#[cfg(test)]
mod tests {
    use super::parse_valid_paths;

    #[test]
    fn parse_valid_paths_ignores_empty_and_comments() {
        let input = r#"
        # comment
        /posts/hello
        /posts/world
        "#
        .trim();
        let paths = parse_valid_paths(input);
        assert!(paths.contains("/posts/hello"));
        assert!(paths.contains("/posts/world"));
        assert_eq!(paths.len(), 2);
    }

    #[test]
    fn parse_valid_paths_skips_non_paths() {
        let input = "posts/hello\n/ok\n";
        let paths = parse_valid_paths(input);
        assert!(paths.contains("/ok"));
        assert_eq!(paths.len(), 1);
    }
}
