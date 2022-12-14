use itertools::Itertools;
use thiserror::Error;
use url::{ParseError, Url};

#[derive(Debug)]
struct ExpandEnvironment {
    remainder: Vec<String>,
}

fn expand(input: &str, environment: ExpandEnvironment) -> String {
    let mut url = Url::parse(input).unwrap();

    if !environment.remainder.is_empty() {
        url.set_path(&format!(
            "{}/{}",
            url.path().trim_end_matches('/'),
            environment.remainder.join("/")
        ));
    }

    url.to_string()
}

#[derive(Error, Debug, Clone, Copy, PartialEq, Eq)]
pub enum GolinkError {
    #[error("Input unable to be parsed as URL")]
    UrlParseError(#[from] ParseError),

    #[error("No first path segment")]
    NoFirstPathSegment,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GolinkResolution {
    MetadataRequest(String),
    RedirectRequest(String),
}

pub fn resolve(
    input: &str,
    lookup: &dyn Fn(&str) -> Option<String>,
) -> Result<GolinkResolution, GolinkError> {
    let url = Url::parse(input)?;
    let mut segments = url.path_segments().unwrap();
    let short = segments
        .next()
        .ok_or(GolinkError::NoFirstPathSegment)?
        .to_ascii_lowercase()
        .replace('-', "");

    if {
        let this = &url.path().chars().last();
        let f = |char| char == &'+';
        matches!(this, Some(x) if f(x))
    } {
        return Ok(GolinkResolution::MetadataRequest(
            short.trim_end_matches('+').to_owned(),
        ));
    }

    let remainder = segments.map(|s| s.to_owned()).collect_vec();

    let lookup_value = lookup(&short);

    Ok(GolinkResolution::RedirectRequest(expand(
        &lookup_value.unwrap(),
        ExpandEnvironment { remainder },
    )))
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    fn lookup(input: &str) -> Option<String> {
        if input == "test" {
            return Some("http://example.com".to_string());
        }
        if input == "test2" {
            return Some("http://example.com/test.html?a=b&c[]=d".to_string());
        }
        None
    }

    #[test]
    fn it_works() {
        let computed = resolve("http://go/test", &lookup);
        assert_eq!(
            computed,
            Ok(GolinkResolution::RedirectRequest(
                "http://example.com/".to_string()
            ))
        )
    }

    #[test]
    fn it_works_for_google_maps_url() {
        let computed = resolve("http://go/test2", &lookup);
        assert_eq!(
            computed,
            Ok(GolinkResolution::RedirectRequest(
                "http://example.com/test.html?a=b&c[]=d".to_string()
            ))
        )
    }

    #[test]
    fn it_ignores_case() {
        let computed = resolve("http://go/TEST", &lookup);
        assert_eq!(
            computed,
            Ok(GolinkResolution::RedirectRequest(
                "http://example.com/".to_string()
            ))
        )
    }

    #[test]
    fn it_ignores_hyphens() {
        let computed = resolve("http://go/t-est", &lookup);
        assert_eq!(
            computed,
            Ok(GolinkResolution::RedirectRequest(
                "http://example.com/".to_string()
            ))
        )
    }

    #[test]
    fn it_returns_metadata_request() {
        let computed = resolve("http://go/test+", &lookup);
        assert_eq!(
            computed,
            Ok(GolinkResolution::MetadataRequest("test".to_string()))
        )
    }

    #[test]
    fn it_returns_correct_metadata_request_with_hyphens() {
        let computed = resolve("http://go/tEs-t+", &lookup);
        assert_eq!(
            computed,
            Ok(GolinkResolution::MetadataRequest("test".to_string()))
        )
    }

    #[test]
    fn it_appends_remaining_path_segments() {
        let computed = resolve("http://go/test/a/b/c", &lookup);
        assert_eq!(
            computed,
            Ok(GolinkResolution::RedirectRequest(
                "http://example.com/a/b/c".to_string()
            ))
        )
    }

    #[test]
    fn it_appends_remaining_path_segments_for_maps_url() {
        let computed = resolve("http://go/test2/a/b/c", &lookup);
        assert_eq!(
            computed,
            Ok(GolinkResolution::RedirectRequest(
                "http://example.com/test.html/a/b/c?a=b&c[]=d".to_string()
            ))
        )
    }
}
