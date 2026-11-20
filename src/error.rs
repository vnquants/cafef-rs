//! Error type for the CafeF client.

use thiserror::Error;

/// Everything that can go wrong when talking to CafeF.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum CafefError {
    /// Transport error: connection, timeout, DNS, TLS, body decoding.
    #[error("HTTP request failed: {0}")]
    Http(#[from] wreq::Error),

    /// CafeF answered with an HTML page where JSON was expected: an error page,
    /// a redirect to `/404.aspx`, a login page or a bot challenge.
    #[error(
        "CafeF returned an HTML page instead of data (HTTP {status}, title {title:?}, challenge: {challenge}) from {url}"
    )]
    HtmlPage {
        /// Requested URL.
        url: String,
        /// HTTP status code of the HTML response.
        status: u16,
        /// Contents of `<title>`, if any.
        title: Option<String>,
        /// True when the page looks like an anti-bot challenge
        /// (Cloudflare "Just a moment...", captcha, "Access denied").
        challenge: bool,
    },

    /// HTTP 401/403: the endpoint wants a login or has blocked this client.
    /// No CafeF endpoint used by this crate needed a login on 2026-11-19.
    #[error("access denied (HTTP {status}) at {url}: login required or client blocked")]
    AccessDenied {
        /// Requested URL.
        url: String,
        /// HTTP status code.
        status: u16,
    },

    /// HTTP 400 from `apiweb.cafef.vn` (ASP.NET validation problem details),
    /// e.g. an unknown `TypeTime` value.
    #[error("CafeF rejected the request parameters at {url}: {message}")]
    InvalidRequest {
        /// Requested URL.
        url: String,
        /// Validation messages joined into one line.
        message: String,
    },

    /// Any other non-success HTTP status. `apiweb.cafef.vn` answers an unknown
    /// symbol with HTTP 500 and an empty body.
    #[error("HTTP {status} from {url}: {body}")]
    Status {
        /// Requested URL.
        url: String,
        /// HTTP status code.
        status: u16,
        /// Start of the response body (may be empty).
        body: String,
    },

    /// CafeF returned its JSON envelope with `Success: false` / `isSuccess: false`.
    #[error("CafeF reported failure at {url}: {message}")]
    Api {
        /// Requested URL.
        url: String,
        /// `Message` / `errors` from the envelope.
        message: String,
    },

    /// HTTP 200 but the body is not the JSON shape this crate expects.
    #[error("unexpected response from {url}: {reason}; body starts with: {snippet}")]
    UnexpectedResponse {
        /// Requested URL.
        url: String,
        /// What did not match (serde error message or a description).
        reason: String,
        /// First characters of the body, for diagnostics.
        snippet: String,
    },

    /// The data exists on the web page but this crate does not offer it in that
    /// shape (for example quarterly financial ratios: CafeF only has annual ones).
    #[error("not available from CafeF: {0}")]
    Unsupported(String),

    /// Empty or malformed ticker.
    #[error("invalid symbol: {0:?}")]
    InvalidSymbol(String),

    /// Local file-system error while saving a download.
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
}

impl CafefError {
    /// True when CafeF served an anti-bot challenge or denied access, i.e. the
    /// request should not simply be retried.
    pub fn is_blocked(&self) -> bool {
        matches!(
            self,
            Self::HtmlPage {
                challenge: true,
                ..
            } | Self::AccessDenied { .. }
        )
    }
}
