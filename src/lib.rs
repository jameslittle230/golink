//! # Golink
//!
//! The Golink crate is an engine for resolving URLs for link shortening services.
//! You provide a link to expand and a function for mapping short URLs to long URLs,
//! and this crate will:
//!
//! - **Normalize your input to ignore case and hyphenation**: `http://go/My-Service`
//! and `http://go/myservice` are treated as the same input into your mapping function
//!
//! - **Append secondary paths to your resolved URL**: if your mapping function returns
//! `http://example.com` for the given shortlink `foo`, then a request to `http://go/foo/bar/baz`
//! will resolve to `http://example.com/foo/bar/baz`
//!
//! - **Apply templating, when applicable**: Using a simple templating language, your long URLs
//! can powerfully place remaining path segments in your URL ad-hoc and provide a fallback
//! value when there are no remaining path segments. For example, if your mapping function
//! returns for the given shortlink `prs` the following URL:
//!
//!     ```text
//!     https://github.com/pulls?q=is:open+is:pr+review-requested:{{ if path }}{ path }{{ else }}@me{{ endif }}+archived:false
//!     ```
//!
//!     then a request to `http://go/prs` returns the URL to all Github PRs to which
//!     you are assigned:
//!
//!     ```text
//!     https://github.com/pulls?q=is:open+is:pr+review-requested:@me+archived:false
//!     ```
//!
//!     and a request to `http://go/prs/jameslittle230` returns the URL to all
//!     Github PRs to which I ([@jameslittle230](https://github.com/jameslittle230))
//!     am assigned:
//!
//!     ```text
//!     https://github.com/pulls?q=is:open+is:pr+review-requested:jameslittle230+archived:false
//!     ```
//!
//! This resolver performs all the functionality described in [Tailscale's Golink
//! project](https://tailscale.com/blog/golink/)
//!
//! This crate doesn't provide a web service or an interface for creating shortened links;
//! it only provides an algorithm for resolving short URLs to long URLs.
//!
//! ## Usage
//!
//! The Golink crate doesn't care how you store or retrieve long URLs given a short URL;
//! you can store them in memory, in a database, or on disk, as long as they are retrievable
//! from within a closure you pass into the `resolve()` function:
//!
//! ```rust
//! fn lookup(input: &str) -> Option<String> {
//!     if input == "foo" {
//!         return Some("http://example.com".to_string());
//!     }
//!     None
//! }
//!
//! let resolved = golink::resolve("http://go/foo", &lookup)
//!
//! match computed {
//!    Ok(GolinkResolution::RedirectRequest(url)) => {
//!        // Redirect to `url`
//!    }
//!
//!    Ok(GolinkResolution::MetadataRequest(key)) => {
//!        // `key` is the original shortlink.
//!        // Return JSON that displays metadata/analytics about `key`
//!    }
//!
//!    Err(e) => {
//!        // Return a 400 error to the user, with a message based on `e`
//!    }
//! }
//! ```

use itertools::Itertools;
use serde::Serialize;
use thiserror::Error;
use tinytemplate::TinyTemplate;
use url::{ParseError, Url};

#[derive(Debug, Serialize)]
struct ExpandEnvironment {
    path: String,
}

fn expand(input: &str, environment: ExpandEnvironment) -> Result<String, GolinkError> {
    let mut tt = TinyTemplate::new();
    tt.add_template("url_input", input)?;
    let rendered = tt.render("url_input", &environment)?;

    // If rendering didn't result in a different output, assume there is no render
    // syntax in our long value, and append the incoming remainder path onto the
    // expanded URL's path
    if input == rendered {
        let mut url = Url::parse(input)?;

        if !environment.path.is_empty() {
            url.set_path(&vec![url.path().trim_end_matches('/'), &environment.path].join("/"));
        }

        Ok(url.to_string())
    } else {
        let url = Url::parse(&rendered)?;
        Ok(url.to_string())
    }
}

#[derive(Error, Debug, Clone, PartialEq, Eq)]
pub enum GolinkError {
    #[error("String could not be parsed as URL")]
    UrlParseError(#[from] ParseError),

    #[error("Could not pull path segments from the input value")]
    InvalidInputUrl,

    #[error("No first path segment")]
    NoFirstPathSegment,

    #[error("Could not parse template correctly")]
    ImproperTemplate(String),

    #[error("Key {0} not found in lookup function")]
    NotFound(String),
}

impl From<tinytemplate::error::Error> for GolinkError {
    fn from(tt_error: tinytemplate::error::Error) -> Self {
        GolinkError::ImproperTemplate(tt_error.to_string())
    }
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
    let mut segments = url.path_segments().ok_or(GolinkError::InvalidInputUrl)?;
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

    let remainder = segments.join("/");

    let lookup_value = lookup(&short).ok_or_else(|| GolinkError::NotFound(short.clone()))?;

    let expansion = expand(&lookup_value, ExpandEnvironment { path: remainder })?;

    Ok(GolinkResolution::RedirectRequest(expansion))
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
        if input == "prs" {
            return Some("https://github.com/pulls?q=is:open+is:pr+review-requested:{{ if path }}{ path }{{ else }}@me{{ endif }}+archived:false".to_string());
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

    #[test]
    fn it_uses_path_in_template() {
        let computed = resolve("http://go/prs/jameslittle230", &lookup);
        assert_eq!(
            computed,
            Ok(GolinkResolution::RedirectRequest(
                "https://github.com/pulls?q=is:open+is:pr+review-requested:jameslittle230+archived:false".to_string()
            ))
        )
    }

    #[test]
    fn it_uses_fallback_in_template() {
        let computed = resolve("http://go/prs", &lookup);
        assert_eq!(
            computed,
            Ok(GolinkResolution::RedirectRequest(
                "https://github.com/pulls?q=is:open+is:pr+review-requested:@me+archived:false"
                    .to_string()
            ))
        )
    }

    #[test]
    fn it_uses_fallback_in_template_with_trailing_slash() {
        let computed = resolve("http://go/prs/", &lookup);
        assert_eq!(
            computed,
            Ok(GolinkResolution::RedirectRequest(
                "https://github.com/pulls?q=is:open+is:pr+review-requested:@me+archived:false"
                    .to_string()
            ))
        )
    }

    #[test]
    #[ignore = "TBD"]
    fn it_fails_with_invalid_input_url() {
        let computed = resolve("http://go/", &lookup);
        assert_eq!(computed, Err(GolinkError::InvalidInputUrl))
    }
}
