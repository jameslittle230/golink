# Golink

The Golink crate is an engine for resolving URLs for link shortening services.
You provide a link to expand and a function for mapping short URLs to long URLs,
and this crate will:

- **Normalize your input to ignore case and hyphenation**: `http://go/My-Service`
  and `http://go/myservice` are treated as the same input into your mapping function

- **Append secondary paths to your resolved URL**: if your mapping function returns
  `http://example.com` for the given shortlink `foo`, then a request to `http://go/foo/bar/baz`
  will resolve to `http://example.com/foo/bar/baz`

- **Apply templating, when applicable**: Using a simple templating language, your long URLs
  can powerfully place remaining path segments in your URL ad-hoc and provide a fallback
  value when there are no remaining path segments. For example, if your mapping function
  returns for the given shortlink `prs` the following URL:

  ```text
  https://github.com/pulls?q=is:open+is:pr+review-requested:{{ if path }}{ path }{{ else }}@me{{ endif }}+archived:false
  ```

  then a request to `http://go/prs` returns the URL to all Github PRs to which
  you are assigned:

  ```text
  https://github.com/pulls?q=is:open+is:pr+review-requested:@me+archived:false
  ```

  and a request to `http://go/prs/jameslittle230` returns the URL to all
  Github PRs to which I ([@jameslittle230](https://github.com/jameslittle230))
  am assigned:

  ```text
  https://github.com/pulls?q=is:open+is:pr+review-requested:jameslittle230+archived:false
  ```

This resolver performs all the functionality described in [Tailscale's Golink
project](https://tailscale.com/blog/golink/)

This crate doesn't provide a web service or an interface for creating shortened links;
it only provides an algorithm for resolving short URLs to long URLs.

## Usage

The Golink crate doesn't care how you store or retrieve long URLs given a short URL;
you can store them in memory, in a database, or on disk, as long as they are retrievable
from within a closure you pass into the `resolve()` function:

```rust
fn lookup(input: &str) -> Option<String> {
    if input == "foo" {
        return Some("http://example.com".to_string());
    }
    None
}

let resolved = golink::resolve("http://go/foo", &lookup)

match computed {
   Ok(GolinkResolution::RedirectRequest(url)) => {
       // Redirect to `url`
   }

   Ok(GolinkResolution::MetadataRequest(key)) => {
       // `key` is the original shortlink.
       // Return JSON that displays metadata/analytics about `key`
   }

   Err(e) => {
       // Return a 400 error to the user, with a message based on `e`
   }
}
```
