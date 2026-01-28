//! # Golink
//!
//! The Golink crate is an engine for resolving URLs for link shortening services.
//! You provide a link to expand and a function for mapping short URLs to long URLs,
//! and this crate will:
//!
//! - **Normalize your input to ignore case and hyphenation**: `http://go/My-Service`
//!   and `http://go/myservice` are treated as the same input into your mapping function
//!
//! - **Append secondary paths to your resolved URL**: if your mapping function returns
//!   `http://example.com` for the given shortlink `foo`, then a request to `http://go/foo/bar/baz`
//!   will resolve to `http://example.com/foo/bar/baz`
//!
//! - **Apply templating, when applicable**: Using a simple templating language, your long URLs
//!   can powerfully place remaining path segments in your URL ad-hoc and provide a fallback
//!   value when there are no remaining path segments. For example, if your mapping function
//!   returns for the given shortlink `prs` the following URL:
//!
//!   ```text
//!   https://github.com/pulls?q=is:open+is:pr+review-requested:{{ if path }}{ path }{{ else }}@me{{ endif }}+archived:false
//!   ```
//!
//!   then a request to `http://go/prs` returns the URL to all Github PRs to which
//!   you are assigned:
//!
//!   ```text
//!   https://github.com/pulls?q=is:open+is:pr+review-requested:@me+archived:false
//!   ```
//!
//!   and a request to `http://go/prs/jameslittle230` returns the URL to all
//!   Github PRs to which I ([@jameslittle230](https://github.com/jameslittle230))
//!   am assigned:
//!
//!   ```text
//!   https://github.com/pulls?q=is:open+is:pr+review-requested:jameslittle230+archived:false
//!   ```
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
//! from within a closure you pass into the `resolve()` or `resolve_async()` function.
//!
//! ### Synchronous API
//!
//! ```rust
//! fn lookup(input: &str) -> Option<String> {
//!     if input == "foo" {
//!         return Some("http://example.com".to_string());
//!     }
//!     None
//! }
//!
//! let resolved = golink::resolve("/foo", &lookup);
//!  //         or golink::resolve("foo", &lookup);
//!  //         or golink::resolve("https://example.com/foo", &lookup);
//!
//! match resolved {
//!    Ok(golink::GolinkResolution::RedirectRequest { url, shortlink }) => {
//!        // Redirect to `url`
//!        // If you collect analytics, then increment the click count for `shortlink`
//!    }
//!
//!    Ok(golink::GolinkResolution::MetadataRequest(key)) => {
//!        // `key` is the original shortlink.
//!        // Return JSON that displays metadata/analytics about `key`
//!    }
//!
//!    Err(e) => {
//!        // Return an error to the user based on the type of error (see `GolinkError` for more)
//!    }
//! }
//! ```
//!
//! ### Asynchronous API
//!
//! For async contexts (e.g., database lookups), use `resolve_async()`:
//!
//! ```rust
//! # async fn example() {
//! let resolved = golink::resolve_async("/foo", |input| {
//!     let input = input.to_string();
//!     async move {
//!         // Could be an async database query
//!         if input == "foo" {
//!             return Some("http://example.com".to_string());
//!         }
//!         None
//!     }
//! }).await;
//!
//! match resolved {
//!    Ok(golink::GolinkResolution::RedirectRequest { url, shortlink }) => {
//!        // Redirect to `url`
//!        // Optionally use `shortlink` for analytics
//!    }
//!    Ok(golink::GolinkResolution::MetadataRequest(key)) => {
//!        // Return metadata about `key`
//!    }
//!    Err(e) => {
//!        // Handle error
//!    }
//! }
//! # }
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

fn expand(input: &str, environment: &ExpandEnvironment) -> Result<String, GolinkError> {
    let mut tt = TinyTemplate::new();
    tt.add_template("url_input", input)?;
    let rendered = tt.render("url_input", environment)?;

    // If rendering didn't result in a different output, assume there is no render
    // syntax in our long value and instead append the incoming remainder path onto the
    // expanded URL's path
    if input == rendered {
        if let Ok(mut url) = Url::parse(input) {
            if !environment.path.is_empty() {
                let base_path = url.path().trim_end_matches('/');
                url.set_path(&format!("{base_path}/{}", environment.path));
            }
            Ok(url.to_string())
        } else if environment.path.is_empty() {
            Ok(rendered)
        } else {
            Ok(format!("{rendered}/{}", environment.path))
        }
    } else {
        Ok(rendered)
    }
}

/// Errors that can occur during shortlink resolution.
///
/// These errors are designed to map naturally to HTTP status codes:
/// - `InvalidInput` → HTTP 400 Bad Request
/// - `NotFound` → HTTP 404 Not Found
/// - `TemplateError` → HTTP 500 Internal Server Error
///
/// # Example: Mapping to HTTP Status Codes
///
/// ```
/// use golink::{resolve, GolinkError};
///
/// fn lookup(key: &str) -> Option<String> {
///     // Your lookup implementation
/// #   None
/// }
///
/// fn handle_request(path: &str) -> (u16, String) {
///     match resolve(path, lookup) {
///         Ok(resolution) => {
///             // Handle successful resolution
/// #           (200, "OK".to_string())
///         }
///         Err(GolinkError::InvalidInput) => {
///             (400, format!("Invalid shortlink {path}"))
///         }
///         Err(GolinkError::NotFound(shortlink)) => {
///             (404, format!("Shortlink '{shortlink}' not found"))
///         }
///         Err(GolinkError::TemplateError(msg)) => {
///             // Log this error - it indicates a data integrity problem
///             eprintln!("Template error: {msg}");
///             (500, "Internal Server Error".to_string())
///         }
///     }
/// }
/// ```
#[derive(Error, Debug, Clone, PartialEq, Eq)]
pub enum GolinkError {
    /// The input shortlink is invalid or malformed.
    ///
    /// This covers cases like:
    /// - Strings that don't make sense as a URL path (e.g. `a:b`)
    /// - Empty input, or empty input after normalization
    ///
    /// **Recommended HTTP status: 400 Bad Request**
    #[error("Invalid input")]
    InvalidInput,

    /// The shortlink was not found in the lookup function.
    ///
    /// The lookup function returned `None` for the given shortlink.
    /// The contained `String` is the normalized shortlink that was not found.
    ///
    /// **Recommended HTTP status: 404 Not Found**
    #[error("Shortlink '{0}' not found")]
    NotFound(String),

    /// The long URL contains invalid template syntax.
    ///
    /// This indicates a configuration or data integrity problem - the stored long URL
    /// has malformed template syntax. This is not the user's fault.
    ///
    /// **Recommended HTTP status: 500 Internal Server Error**
    #[error("Template error: {0}")]
    TemplateError(String),
}

impl From<ParseError> for GolinkError {
    fn from(_: ParseError) -> Self {
        GolinkError::InvalidInput
    }
}

impl From<tinytemplate::error::Error> for GolinkError {
    fn from(tt_error: tinytemplate::error::Error) -> Self {
        GolinkError::TemplateError(tt_error.to_string())
    }
}

/// The result of resolving a short URL.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GolinkResolution {
    /// A request for metadata about a shortlink (triggered by a trailing '+').
    ///
    /// The contained `String` is the normalized shortlink that metadata was requested for.
    /// You should return analytics, usage statistics, or other metadata about this shortlink.
    MetadataRequest(String),

    /// A request to redirect to the expanded URL.
    ///
    /// Contains the expanded URL to redirect to and the normalized shortlink that was used.
    RedirectRequest {
        /// The fully expanded URL to redirect the user to.
        url: String,
        /// The normalized shortlink that was resolved.
        shortlink: String,
    },
}

/// Normalizes a shortlink by extracting the first path segment and converting to lowercase,
/// removing hyphens and spaces.
///
/// This function is useful when storing new shortlinks in your database. By normalizing
/// the user-provided shortlink before saving it, you ensure that lookups are consistent
/// regardless of how users type the shortlink (e.g., "My-Service", "my-service", and
/// "myservice" all normalize to the same value).
///
/// If the input contains slashes, only the first path segment is used, matching the
/// behavior of `resolve()` and `resolve_async()`. This prevents accidentally creating
/// shortlinks that won't match during resolution.
///
/// This applies the same normalization rules used internally by `resolve()` and
/// `resolve_async()`.
///
/// # Examples
///
/// ```
/// // When a user creates a new shortlink, normalize it before storing
/// let user_input = "My-Service";
/// let normalized = golink::normalize_shortlink(user_input);
/// // Store `normalized` ("myservice") in your database (make sure to
/// // check for duplicates!)
///
/// assert_eq!(golink::normalize_shortlink("My-Service"), "myservice");
/// assert_eq!(golink::normalize_shortlink("FOO"), "foo");
/// assert_eq!(golink::normalize_shortlink("my service"), "myservice");
///
/// // Extracts only the first path segment
/// assert_eq!(golink::normalize_shortlink("foo/bar"), "foo");
/// assert_eq!(golink::normalize_shortlink("/foo/bar/baz"), "foo");
/// assert_eq!(golink::normalize_shortlink("My-Service/docs"), "myservice");
/// ```
#[must_use]
pub fn normalize_shortlink(input: &str) -> String {
    // Extract first non-empty path segment
    let first_segment = input
        .trim_start_matches('/')
        .split('/')
        .next()
        .unwrap_or("");

    normalize_segment(first_segment)
}

/// Normalizes a single shortlink segment (internal helper).
fn normalize_segment(segment: &str) -> String {
    segment
        .to_ascii_lowercase()
        .replace('-', "")
        .replace("%20", "")
        .replace(' ', "")
}

struct ParsedInput {
    short: String,
    remainder: String,
    is_metadata_request: bool,
}

fn parse_input(input: &str) -> Result<ParsedInput, GolinkError> {
    let url = Url::parse(input).or_else(|_| Url::parse("https://go/")?.join(input))?;
    let mut segments = url.path_segments().ok_or(GolinkError::InvalidInput)?;
    let short = normalize_segment(segments.next().ok_or(GolinkError::InvalidInput)?);

    if short.is_empty() {
        return Err(GolinkError::InvalidInput);
    }

    let is_metadata_request = url.path().ends_with('+');
    let remainder = segments.join("/");

    Ok(ParsedInput {
        short,
        remainder,
        is_metadata_request,
    })
}

/// Resolves a short URL to its expanded form using the provided synchronous lookup function.
///
/// # Examples
///
/// ```
/// use golink::{resolve, GolinkResolution};
///
/// fn lookup(shortlink: &str) -> Option<String> {
///     match shortlink {
///         "home" => Some("https://example.com/".to_string()),
///         "docs" => Some("https://docs.example.com".to_string()),
///         _ => None,
///     }
/// }
///
/// // Basic resolution
/// let result = resolve("/home", lookup).unwrap();
/// match result {
///     GolinkResolution::RedirectRequest { url, shortlink } => {
///         assert_eq!(url, "https://example.com/");
///         assert_eq!(shortlink, "home");
///     }
///     _ => panic!("Expected RedirectRequest"),
/// }
///
/// // Resolution with path appending
/// let result = resolve("/docs/getting-started", lookup).unwrap();
/// match result {
///     GolinkResolution::RedirectRequest { url, .. } => {
///         assert_eq!(url, "https://docs.example.com/getting-started");
///     }
///     _ => panic!("Expected RedirectRequest"),
/// }
///
/// // Metadata request (trailing '+')
/// let result = resolve("/home+", lookup).unwrap();
/// match result {
///     GolinkResolution::MetadataRequest(key) => {
///         assert_eq!(key, "home");
///     }
///     _ => panic!("Expected MetadataRequest"),
/// }
/// ```
///
/// # Errors
///
/// - `InvalidInput`: The input URL is malformed, has no path segments, or the shortlink is empty
/// - `NotFound`: The lookup function returned `None` for the shortlink
/// - `TemplateError`: The long URL contains invalid template syntax
pub fn resolve<F>(input: &str, lookup: F) -> Result<GolinkResolution, GolinkError>
where
    F: Fn(&str) -> Option<String>,
{
    let parsed = parse_input(input)?;

    if parsed.is_metadata_request {
        return Ok(GolinkResolution::MetadataRequest(
            parsed.short.trim_end_matches('+').to_string(),
        ));
    }

    let lookup_value =
        lookup(&parsed.short).ok_or_else(|| GolinkError::NotFound(parsed.short.clone()))?;

    let expansion = expand(
        &lookup_value,
        &ExpandEnvironment {
            path: parsed.remainder,
        },
    )?;

    Ok(GolinkResolution::RedirectRequest {
        url: expansion,
        shortlink: parsed.short,
    })
}

/// Resolves a short URL to its expanded form using the provided asynchronous lookup function.
///
/// This is useful when your shortlink lookup requires async operations like database queries
/// or HTTP requests.
///
/// # Examples
///
/// ```
/// use golink::{resolve_async, GolinkResolution};
///
/// # async fn example() {
/// // Basic async resolution with closure
/// let result = resolve_async("/home", |shortlink| {
///     let shortlink = shortlink.to_string();
///     async move {
///         match shortlink.as_str() {
///             "home" => Some("https://example.com/".to_string()),
///             "docs" => Some("https://docs.example.com".to_string()),
///             _ => None,
///         }
///     }
/// }).await.unwrap();
///
/// match result {
///     GolinkResolution::RedirectRequest { url, shortlink } => {
///         assert_eq!(url, "https://example.com/");
///         assert_eq!(shortlink, "home");
///     }
///     _ => panic!("Expected RedirectRequest"),
/// }
///
/// // With path appending
/// let result = resolve_async("/docs/api", |shortlink| {
///     let shortlink = shortlink.to_string();
///     async move {
///         // Could be an async database query here
///         match shortlink.as_str() {
///             "docs" => Some("https://docs.example.com".to_string()),
///             _ => None,
///         }
///     }
/// }).await.unwrap();
///
/// match result {
///     GolinkResolution::RedirectRequest { url, .. } => {
///         assert_eq!(url, "https://docs.example.com/api");
///     }
///     _ => panic!("Expected RedirectRequest"),
/// }
/// # }
/// ```
///
/// # Errors
///
/// - `InvalidInput`: The input URL is malformed, has no path segments, or the shortlink is empty
/// - `NotFound`: The lookup function returned `None` for the shortlink
/// - `TemplateError`: The long URL contains invalid template syntax
pub async fn resolve_async<F, Fut>(input: &str, lookup: F) -> Result<GolinkResolution, GolinkError>
where
    F: Fn(&str) -> Fut,
    Fut: std::future::Future<Output = Option<String>>,
{
    let parsed = parse_input(input)?;

    if parsed.is_metadata_request {
        return Ok(GolinkResolution::MetadataRequest(
            parsed.short.trim_end_matches('+').to_string(),
        ));
    }

    let lookup_value = lookup(&parsed.short)
        .await
        .ok_or_else(|| GolinkError::NotFound(parsed.short.clone()))?;

    let expansion = expand(
        &lookup_value,
        &ExpandEnvironment {
            path: parsed.remainder,
        },
    )?;

    Ok(GolinkResolution::RedirectRequest {
        url: expansion,
        shortlink: parsed.short,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    fn lookup(input: &str) -> Option<String> {
        if input == "test" {
            return Some("http://example.com/".to_string());
        }
        if input == "test2" {
            return Some("http://example.com/test.html?a=b&c[]=d".to_string());
        }
        if input == "prs" {
            return Some("https://github.com/pulls?q=is:open+is:pr+review-requested:{{ if path }}{ path }{{ else }}@me{{ endif }}+archived:false".to_string());
        }
        if input == "abcd" {
            return Some("efgh".to_string());
        }
        None
    }

    #[test]
    fn it_works() {
        let computed = resolve("/test", &lookup);
        assert_eq!(
            computed,
            Ok(GolinkResolution::RedirectRequest {
                url: "http://example.com/".to_string(),
                shortlink: "test".to_string()
            })
        )
    }

    #[test]
    fn it_works_with_url() {
        let computed = resolve("https://jil.im/test", &lookup);
        assert_eq!(
            computed,
            Ok(GolinkResolution::RedirectRequest {
                url: "http://example.com/".to_string(),
                shortlink: "test".to_string()
            })
        )
    }

    #[test]
    fn it_works_with_no_leading_slash() {
        let computed = resolve("test", &lookup);
        assert_eq!(
            computed,
            Ok(GolinkResolution::RedirectRequest {
                url: "http://example.com/".to_string(),
                shortlink: "test".to_string()
            })
        )
    }

    #[test]
    fn it_works_for_complex_url() {
        let computed = resolve("/test2", &lookup);
        assert_eq!(
            computed,
            Ok(GolinkResolution::RedirectRequest {
                url: "http://example.com/test.html?a=b&c[]=d".to_string(),
                shortlink: "test2".to_string()
            })
        )
    }

    #[test]
    fn it_ignores_case() {
        let computed = resolve("/TEST", &lookup);
        assert_eq!(
            computed,
            Ok(GolinkResolution::RedirectRequest {
                url: "http://example.com/".to_string(),
                shortlink: "test".to_string()
            })
        )
    }

    #[test]
    fn it_ignores_hyphens() {
        let computed = resolve("/t-est", &lookup);
        assert_eq!(
            computed,
            Ok(GolinkResolution::RedirectRequest {
                url: "http://example.com/".to_string(),
                shortlink: "test".to_string()
            })
        )
    }

    #[test]
    fn it_ignores_whitespace() {
        let computed = resolve("/t est", &lookup);
        assert_eq!(
            computed,
            Ok(GolinkResolution::RedirectRequest {
                url: "http://example.com/".to_string(),
                shortlink: "test".to_string()
            })
        )
    }

    #[test]
    fn it_returns_metadata_request() {
        let computed = resolve("/test+", &lookup);
        assert_eq!(
            computed,
            Ok(GolinkResolution::MetadataRequest("test".to_string()))
        )
    }

    #[test]
    fn it_returns_correct_metadata_request_with_hyphens() {
        let computed = resolve("/tEs-t+", &lookup);
        assert_eq!(
            computed,
            Ok(GolinkResolution::MetadataRequest("test".to_string()))
        )
    }

    #[test]
    fn it_does_not_append_remaining_path_segments_with_invalid_resolved_url() {
        let computed = resolve("/abcd/a/b/c", &lookup);
        assert_eq!(
            computed,
            Ok(GolinkResolution::RedirectRequest {
                url: "efgh/a/b/c".to_string(),
                shortlink: "abcd".to_string()
            })
        )
    }

    #[test]
    fn it_appends_remaining_path_segments() {
        let computed = resolve("/test/a/b/c", &lookup);
        assert_eq!(
            computed,
            Ok(GolinkResolution::RedirectRequest {
                url: "http://example.com/a/b/c".to_string(),
                shortlink: "test".to_string()
            })
        )
    }

    #[test]
    fn it_appends_remaining_path_segments_for_maps_url() {
        let computed = resolve("/test2/a/b/c", &lookup);
        assert_eq!(
            computed,
            Ok(GolinkResolution::RedirectRequest {
                url: "http://example.com/test.html/a/b/c?a=b&c[]=d".to_string(),
                shortlink: "test2".to_string()
            })
        )
    }

    #[test]
    fn it_uses_path_in_template() {
        let computed = resolve("/prs/jameslittle230", &lookup);
        assert_eq!(
            computed,
            Ok(GolinkResolution::RedirectRequest {
                url: "https://github.com/pulls?q=is:open+is:pr+review-requested:jameslittle230+archived:false".to_string(),
                shortlink: "prs".to_string()
            })
        )
    }

    #[test]
    fn it_uses_fallback_in_template() {
        let computed = resolve("/prs", &lookup);
        assert_eq!(
            computed,
            Ok(GolinkResolution::RedirectRequest {
                url: "https://github.com/pulls?q=is:open+is:pr+review-requested:@me+archived:false"
                    .to_string(),
                shortlink: "prs".to_string()
            })
        )
    }

    #[test]
    fn it_uses_fallback_in_template_with_trailing_slash() {
        let computed = resolve("/prs/", &lookup);
        assert_eq!(
            computed,
            Ok(GolinkResolution::RedirectRequest {
                url: "https://github.com/pulls?q=is:open+is:pr+review-requested:@me+archived:false"
                    .to_string(),
                shortlink: "prs".to_string()
            })
        )
    }

    #[test]
    fn it_allows_the_long_url_to_not_be_a_valid_url() {
        let computed = resolve("/abcd", &lookup);
        assert_eq!(
            computed,
            Ok(GolinkResolution::RedirectRequest {
                url: "efgh".to_string(),
                shortlink: "abcd".to_string()
            })
        )
    }

    #[test]
    fn normalize_shortlink_extracts_first_segment() {
        assert_eq!(normalize_shortlink("foo/bar"), "foo");
        assert_eq!(normalize_shortlink("/foo/bar/baz"), "foo");
        assert_eq!(normalize_shortlink("My-Service/docs"), "myservice");
        assert_eq!(normalize_shortlink("FOO/BAR"), "foo");
        assert_eq!(normalize_shortlink("my service/other"), "myservice");
    }

    #[test]
    fn it_fails_with_invalid_input_url() {
        let computed = resolve("a:3gb", &lookup);
        assert!(matches!(computed, Err(GolinkError::InvalidInput)));
    }

    #[test]
    fn it_fails_with_empty_string() {
        let computed = resolve("", &lookup);
        assert!(matches!(computed, Err(GolinkError::InvalidInput)));
    }

    #[test]
    fn it_fails_with_whitespace_only_string() {
        let computed = resolve("  \n", &lookup);
        assert!(matches!(computed, Err(GolinkError::InvalidInput)));
    }

    // Async tests
    #[tokio::test]
    async fn async_it_works() {
        let computed = resolve_async("/test", |input| {
            let input = input.to_string();
            async move { lookup(&input) }
        })
        .await;
        assert_eq!(
            computed,
            Ok(GolinkResolution::RedirectRequest {
                url: "http://example.com/".to_string(),
                shortlink: "test".to_string()
            })
        )
    }

    #[tokio::test]
    async fn async_it_appends_remaining_path_segments() {
        let computed = resolve_async("/test/a/b/c", |input| {
            let input = input.to_string();
            async move { lookup(&input) }
        })
        .await;
        assert_eq!(
            computed,
            Ok(GolinkResolution::RedirectRequest {
                url: "http://example.com/a/b/c".to_string(),
                shortlink: "test".to_string()
            })
        )
    }

    #[tokio::test]
    async fn async_it_uses_path_in_template() {
        let computed = resolve_async("/prs/jameslittle230", |input| {
            let input = input.to_string();
            async move { lookup(&input) }
        })
        .await;
        assert_eq!(
            computed,
            Ok(GolinkResolution::RedirectRequest {
                url: "https://github.com/pulls?q=is:open+is:pr+review-requested:jameslittle230+archived:false".to_string(),
                shortlink: "prs".to_string()
            })
        )
    }

    #[tokio::test]
    async fn async_it_returns_metadata_request() {
        let computed = resolve_async("/test+", |input| {
            let input = input.to_string();
            async move { lookup(&input) }
        })
        .await;
        assert_eq!(
            computed,
            Ok(GolinkResolution::MetadataRequest("test".to_string()))
        )
    }
}
