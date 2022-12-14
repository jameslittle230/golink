use itertools::Itertools;
use thiserror::Error;
use url::{ParseError, Url};

#[derive(Debug)]
struct ExpandEnvironment {
    remainder: Vec<String>,
}

fn expand(input: &str, environment: ExpandEnvironment) -> String {
    dbg!(environment);
    input.to_string()
}

#[derive(Error, Debug, Clone, Copy, PartialEq, Eq)]
pub enum GolinkError {
    #[error("Input unable to be parsed as URL")]
    UrlParseError(#[from] ParseError),

    #[error("No first path segment")]
    NoFirstPathSegment,
}

pub fn resolve(
    input: &str,
    lookup: &dyn Fn(&str) -> Option<String>,
) -> Result<String, GolinkError> {
    let url = Url::parse(input)?;
    let mut segments = url.path_segments().unwrap();
    let short = segments
        .next()
        .ok_or(GolinkError::NoFirstPathSegment)?
        .to_ascii_lowercase()
        .replace('-', "");

    let remainder = segments.map(|s| s.to_owned()).collect_vec();

    let lookup_value = lookup(&short);

    Ok(expand(
        &lookup_value.unwrap(),
        ExpandEnvironment { remainder },
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lookup(input: &str) -> Option<String> {
        if input == "test" {
            return Some("http://example.com".to_string());
        }
        None
    }

    #[test]
    fn it_works() {
        let computed = resolve("http://go/test", &lookup);
        assert_eq!(computed, Ok("http://example.com".to_string()))
    }

    #[test]
    fn it_ignores_case() {
        let computed = resolve("http://go/TEST", &lookup);
        assert_eq!(computed, Ok("http://example.com".to_string()))
    }

    #[test]
    fn it_ignores_hyphens() {
        let computed = resolve("http://go/t-est", &lookup);
        assert_eq!(computed, Ok("http://example.com".to_string()))
    }
}
