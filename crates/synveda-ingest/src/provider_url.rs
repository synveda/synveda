//! Closed validation for configured model-provider base URLs.

pub(crate) fn normalise(value: &str) -> Option<String> {
    let parsed = reqwest::Url::parse(value).ok()?;
    if !matches!(parsed.scheme(), "http" | "https")
        || !parsed.has_host()
        || !parsed.username().is_empty()
        || parsed.password().is_some()
        || parsed.query().is_some()
        || parsed.fragment().is_some()
    {
        return None;
    }
    Some(parsed.as_str().trim_end_matches('/').to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn provider_urls_are_absolute_http_origins_without_credentials() {
        assert_eq!(
            normalise("https://models.example/v1/"),
            Some("https://models.example/v1".to_owned())
        );
        for refused in [
            "models.example",
            "file:///tmp/provider",
            "https://user:secret@models.example",
            "https://models.example?credential=secret",
            "https://models.example/#fragment",
        ] {
            assert!(normalise(refused).is_none(), "accepted {refused}");
        }
    }
}
