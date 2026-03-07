/// Utilities for parsing and normalising YouTube URLs.
///
/// Accepted input formats:
/// - `https://www.youtube.com/watch?v=VIDEO_ID`
/// - `https://youtube.com/watch?v=VIDEO_ID&list=...` (extra params ignored)
/// - `https://youtu.be/VIDEO_ID`
/// - `https://youtu.be/VIDEO_ID?si=...` (extra params ignored)
/// - Bare video IDs (11-character alphanumeric strings) are passed through as-is.
use crate::error::HttpError;

/// The canonical YouTube watch URL prefix.
const YOUTUBE_WATCH: &str = "https://www.youtube.com/watch?v=";

/// Extracts the 11-character video ID from any supported YouTube URL,
/// then returns the canonical `https://www.youtube.com/watch?v=<id>` URL.
///
/// # Errors
/// Returns [`HttpError::bad_request`] if the input cannot be recognised as a
/// valid YouTube URL or bare video ID.
pub fn normalise_youtube_url(input: &str) -> Result<String, HttpError> {
    let id = extract_video_id(input.trim())?;
    Ok(format!("{YOUTUBE_WATCH}{id}"))
}

/// Extracts just the video ID from a YouTube URL or bare ID string.
fn extract_video_id(input: &str) -> Result<&str, HttpError> {
    // ── 1. youtu.be short links ───────────────────────────────────────────────
    //   https://youtu.be/VIDEO_ID
    //   https://youtu.be/VIDEO_ID?si=...
    if let Some(rest) = strip_prefix_ci(input, "https://youtu.be/")
        .or_else(|| strip_prefix_ci(input, "http://youtu.be/"))
    {
        let id = rest.split(['?', '&', '#']).next().unwrap_or(rest);
        return validate_id(id, input);
    }

    // ── 2. youtube.com/watch?v= ───────────────────────────────────────────────
    //   https://www.youtube.com/watch?v=VIDEO_ID
    //   https://youtube.com/watch?v=VIDEO_ID&list=...
    if is_youtube_watch_url(input) {
        if let Some(id) = query_param(input, "v") {
            return validate_id(id, input);
        }
    }

    // ── 3. youtube.com/shorts/ ────────────────────────────────────────────────
    //   https://www.youtube.com/shorts/VIDEO_ID
    if let Some(rest) = find_path_segment(input, "shorts") {
        let id = rest.split(['?', '&', '#']).next().unwrap_or(rest);
        return validate_id(id, input);
    }

    // ── 4. Bare video ID (11 alphanumeric/dash/underscore chars) ─────────────
    if looks_like_video_id(input) {
        return Ok(input);
    }

    Err(HttpError::bad_request(format!(
        "could not extract a YouTube video ID from: `{input}`"
    )))
}

// ─── helpers ─────────────────────────────────────────────────────────────────

/// Case-insensitive prefix strip.
fn strip_prefix_ci<'a>(s: &'a str, prefix: &str) -> Option<&'a str> {
    if s.len() >= prefix.len() && s[..prefix.len()].eq_ignore_ascii_case(prefix) {
        Some(&s[prefix.len()..])
    } else {
        None
    }
}

fn is_youtube_watch_url(s: &str) -> bool {
    (s.contains("youtube.com/watch") || s.contains("youtube.com/watch"))
        && s.contains("v=")
}

/// Extracts the value of a query parameter from a raw URL string without
/// pulling in a full URL parser — keeps compile times lean.
fn query_param<'a>(url: &'a str, key: &str) -> Option<&'a str> {
    let query_start = url.find('?')?;
    let query = &url[query_start + 1..];

    for pair in query.split('&') {
        if let Some(eq) = pair.find('=') {
            let k = &pair[..eq];
            let v = &pair[eq + 1..];
            if k == key {
                // Strip any fragment
                let v = v.split('#').next().unwrap_or(v);
                return Some(v);
            }
        }
    }
    None
}

/// Returns the path segment *after* `segment_name`, if present.
fn find_path_segment<'a>(url: &'a str, segment_name: &str) -> Option<&'a str> {
    let needle = format!("/{segment_name}/");
    let pos = url.find(needle.as_str())?;
    Some(&url[pos + needle.len()..])
}

/// A YouTube video ID is exactly 11 URL-safe characters.
fn looks_like_video_id(s: &str) -> bool {
    s.len() == 11 && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

fn validate_id<'a>(id: &'a str, original: &str) -> Result<&'a str, HttpError> {
    if looks_like_video_id(id) {
        Ok(id)
    } else {
        Err(HttpError::bad_request(format!(
            "extracted ID `{id}` from `{original}` does not look like a valid YouTube video ID"
        )))
    }
}

// ─── tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_standard_watch_url() {
        let out = normalise_youtube_url("https://www.youtube.com/watch?v=dQw4w9WgXcQ").unwrap();
        assert_eq!(out, "https://www.youtube.com/watch?v=dQw4w9WgXcQ");
    }

    #[test]
    fn parses_watch_url_with_extra_params() {
        let out =
            normalise_youtube_url("https://youtube.com/watch?v=dQw4w9WgXcQ&list=PL123&index=1")
                .unwrap();
        assert_eq!(out, "https://www.youtube.com/watch?v=dQw4w9WgXcQ");
    }

    #[test]
    fn parses_short_link() {
        let out = normalise_youtube_url("https://youtu.be/dQw4w9WgXcQ").unwrap();
        assert_eq!(out, "https://www.youtube.com/watch?v=dQw4w9WgXcQ");
    }

    #[test]
    fn parses_short_link_with_si_param() {
        let out = normalise_youtube_url("https://youtu.be/dQw4w9WgXcQ?si=abc123").unwrap();
        assert_eq!(out, "https://www.youtube.com/watch?v=dQw4w9WgXcQ");
    }

    #[test]
    fn parses_shorts_url() {
        let out =
            normalise_youtube_url("https://www.youtube.com/shorts/dQw4w9WgXcQ").unwrap();
        assert_eq!(out, "https://www.youtube.com/watch?v=dQw4w9WgXcQ");
    }

    #[test]
    fn passes_bare_id() {
        let out = normalise_youtube_url("dQw4w9WgXcQ").unwrap();
        assert_eq!(out, "https://www.youtube.com/watch?v=dQw4w9WgXcQ");
    }

    #[test]
    fn rejects_garbage() {
        assert!(normalise_youtube_url("https://example.com/foo").is_err());
        assert!(normalise_youtube_url("not-a-valid-id").is_err());
        assert!(normalise_youtube_url("").is_err());
    }
}
