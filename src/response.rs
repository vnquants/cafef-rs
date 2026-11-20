//! Turning raw HTTP responses into data or a precise [`CafefError`].

use serde::de::DeserializeOwned;

use crate::error::CafefError;

const SNIPPET_CHARS: usize = 300;

/// Decide whether a text response is usable data.
///
/// Order matters: an HTML body is reported as [`CafefError::HtmlPage`] whatever
/// the status (CafeF redirects unknown handlers to `/404.aspx` with a small
/// HTML page), then 401/403, 400 validation errors, other statuses, and empty
/// bodies.
pub fn check_text_response(url: &str, status: u16, body: String) -> Result<String, CafefError> {
    if looks_like_html(&body) {
        let title = html_title(&body);
        let challenge = is_challenge(title.as_deref(), &body);
        return Err(CafefError::HtmlPage {
            url: url.to_string(),
            status,
            title,
            challenge,
        });
    }
    match status {
        200..=299 => {}
        401 | 403 => {
            return Err(CafefError::AccessDenied {
                url: url.to_string(),
                status,
            });
        }
        400 => {
            return Err(CafefError::InvalidRequest {
                url: url.to_string(),
                message: problem_message(&body).unwrap_or_else(|| snippet(&body)),
            });
        }
        _ => {
            return Err(CafefError::Status {
                url: url.to_string(),
                status,
                body: snippet(&body),
            });
        }
    }
    if body.trim().is_empty() {
        return Err(CafefError::UnexpectedResponse {
            url: url.to_string(),
            reason: "empty body".to_string(),
            snippet: String::new(),
        });
    }
    Ok(body)
}

/// Deserialize JSON, reporting failures as [`CafefError::UnexpectedResponse`]
/// (or [`CafefError::HtmlPage`] if the body is HTML after all).
pub fn parse_json<T: DeserializeOwned>(url: &str, body: &str) -> Result<T, CafefError> {
    let trimmed = body.trim_start_matches('\u{feff}');
    serde_json::from_str(trimmed).map_err(|e| {
        if looks_like_html(trimmed) {
            let title = html_title(trimmed);
            let challenge = is_challenge(title.as_deref(), trimmed);
            CafefError::HtmlPage {
                url: url.to_string(),
                status: 200,
                title,
                challenge,
            }
        } else {
            CafefError::UnexpectedResponse {
                url: url.to_string(),
                reason: e.to_string(),
                snippet: snippet(trimmed),
            }
        }
    })
}

/// True when the body starts like an HTML document.
pub fn looks_like_html(body: &str) -> bool {
    let head: String = body
        .trim_start_matches('\u{feff}')
        .trim_start()
        .chars()
        .take(64)
        .collect::<String>()
        .to_ascii_lowercase();
    head.starts_with("<!doctype")
        || head.starts_with("<html")
        || head.starts_with("<head")
        || head.starts_with("<body")
        || head.starts_with("<?xml")
        || head.starts_with("<!--")
        || head.starts_with("<script")
        || head.starts_with("<div")
}

/// Text of the first `<title>` element.
pub fn html_title(body: &str) -> Option<String> {
    let lower = body.to_ascii_lowercase();
    let open = lower.find("<title")?;
    let start = open + lower[open..].find('>')? + 1;
    let end = start + lower[start..].find("</title")?;
    let title = body.get(start..end)?.split_whitespace().collect::<Vec<_>>();
    let title = title.join(" ");
    (!title.is_empty()).then_some(title)
}

/// Heuristic for anti-bot pages (Cloudflare, generic WAF/captcha pages).
pub fn is_challenge(title: Option<&str>, body: &str) -> bool {
    const TITLES: [&str; 5] = [
        "just a moment",
        "attention required",
        "access denied",
        "checking your browser",
        "ddos",
    ];
    const MARKERS: [&str; 6] = [
        "cf-challenge",
        "challenge-platform",
        "cf_chl_",
        "captcha",
        "turnstile",
        "verify you are human",
    ];
    let title = title.unwrap_or_default().to_ascii_lowercase();
    if TITLES.iter().any(|t| title.contains(t)) {
        return true;
    }
    let lower = body.to_ascii_lowercase();
    MARKERS.iter().any(|m| lower.contains(m))
}

/// Messages from an ASP.NET `application/problem+json` body, joined.
fn problem_message(body: &str) -> Option<String> {
    let v: serde_json::Value = serde_json::from_str(body).ok()?;
    let mut parts = Vec::new();
    if let Some(title) = v.get("title").and_then(|t| t.as_str()) {
        parts.push(title.to_string());
    }
    if let Some(errors) = v.get("errors").and_then(|e| e.as_object()) {
        for (field, msgs) in errors {
            let msgs = match msgs {
                serde_json::Value::Array(a) => a
                    .iter()
                    .map(|m| m.as_str().map_or_else(|| m.to_string(), str::to_string))
                    .collect::<Vec<_>>()
                    .join("; "),
                other => other.to_string(),
            };
            parts.push(format!("{field}: {msgs}"));
        }
    }
    (!parts.is_empty()).then(|| parts.join(" | "))
}

/// First characters of a body, on one line.
pub(crate) fn snippet(body: &str) -> String {
    body.chars()
        .take(SNIPPET_CHARS)
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn html_detection_and_title() {
        let page = "\r\n<html><head><title>Object moved</title></head><body>x</body></html>";
        assert!(looks_like_html(page));
        assert_eq!(html_title(page).as_deref(), Some("Object moved"));
        assert!(!looks_like_html(r#"{"Data":[]}"#));
        assert!(!looks_like_html(""));
        assert_eq!(html_title("<html><title> </title></html>"), None);
    }

    #[test]
    fn challenge_detection() {
        assert!(is_challenge(Some("Just a moment..."), ""));
        assert!(is_challenge(None, "<div id=\"cf-challenge-running\">"));
        assert!(!is_challenge(Some("Object moved"), "<h2>Object moved</h2>"));
    }

    #[test]
    fn status_mapping() {
        let ok = check_text_response("u", 200, "{}".into());
        assert_eq!(ok.unwrap(), "{}");

        let err = check_text_response("u", 500, String::new()).unwrap_err();
        assert!(matches!(err, CafefError::Status { status: 500, .. }));

        let err = check_text_response("u", 403, "denied".into()).unwrap_err();
        assert!(matches!(err, CafefError::AccessDenied { status: 403, .. }));
        assert!(err.is_blocked());

        let err = check_text_response("u", 200, "  ".into()).unwrap_err();
        assert!(matches!(err, CafefError::UnexpectedResponse { .. }));

        let html = "<!DOCTYPE html><html><head><title>Just a moment...</title></head></html>";
        let err = check_text_response("u", 503, html.into()).unwrap_err();
        assert!(matches!(
            err,
            CafefError::HtmlPage {
                status: 503,
                challenge: true,
                ..
            }
        ));
    }

    #[test]
    fn problem_details() {
        let body = r#"{"type":"x","title":"One or more validation errors occurred.","status":400,"errors":{"TypeTime":["The value 'XYZ' is not valid."]}}"#;
        let err = check_text_response("u", 400, body.into()).unwrap_err();
        let CafefError::InvalidRequest { message, .. } = err else {
            panic!("expected InvalidRequest");
        };
        assert!(message.contains("TypeTime: The value 'XYZ' is not valid."));
    }

    #[test]
    fn json_errors() {
        let err = parse_json::<serde_json::Value>("u", "<html><title>x</title>").unwrap_err();
        assert!(matches!(err, CafefError::HtmlPage { .. }));
        let err = parse_json::<Vec<u8>>("u", r#"{"a":1}"#).unwrap_err();
        assert!(matches!(err, CafefError::UnexpectedResponse { .. }));
        let v: serde_json::Value = parse_json("u", "\u{feff}{\"a\":1}").unwrap();
        assert_eq!(v["a"], 1);
    }
}
