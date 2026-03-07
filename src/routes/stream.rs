use axum::{
    body::Body,
    extract::{Query, State},
    http::{HeaderMap, HeaderValue, StatusCode, header},
    response::{IntoResponse, Response},
};
use bytes::Bytes;
use futures::future;
use serde::Deserialize;
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;
use tracing::info;
use utoipa::IntoParams;
use yt_dlp::VideoSelection as _;
use yt_dlp::model::format::{Extension, Format};

use crate::{
    error::{AppError, HttpError},
    middleware::AuthUser,
    state::AppState,
    youtube::normalise_youtube_url,
};

/// Minimum number of parallel Range chunks.
const MIN_CHUNKS: u64 = 4;
/// Maximum number of parallel Range chunks.
const MAX_CHUNKS: u64 = 32;

/// Returns the number of parallel Range chunks to use, scaled to the number of
/// logical CPU cores available on this machine (clamped to [MIN_CHUNKS, MAX_CHUNKS]).
fn parallel_chunks() -> u64 {
    let cores = std::thread::available_parallelism()
        .map(|n| n.get() as u64)
        .unwrap_or(4);
    cores.clamp(MIN_CHUNKS, MAX_CHUNKS)
}

/// Minimum file size (bytes) before we bother splitting into chunks.
/// Below this threshold a single request is faster (less overhead).
const MIN_CHUNK_SIZE: u64 = 256 * 1024; // 256 KB

/// Query parameters for `GET /stream`.
#[derive(Debug, Deserialize, IntoParams)]
pub struct StreamQuery {
    /// The YouTube URL or video ID to stream.
    pub url: String,
}

/// `GET /stream?url=<youtube_url>`
///
/// Resolves the best audio-only format from YouTube using yt-dlp, then
/// fetches the CDN audio in parallel Range chunks and streams the assembled
/// result to the client.
/// Requires a valid JWT (`Authorization: Bearer <token>`).
#[utoipa::path(
    get,
    path = "/stream",
    tag  = "youtube",
    security(("bearer_auth" = [])),
    params(StreamQuery),
    responses(
        (status = 200,  description = "Audio stream (binary)",         content_type = "audio/webm"),
        (status = 400,  description = "Invalid YouTube URL"),
        (status = 401,  description = "Unauthorized"),
        (status = 422,  description = "No audio format available"),
        (status = 502,  description = "CDN upstream error"),
        (status = 500,  description = "Internal server error"),
    )
)]
#[tracing::instrument(skip(state), fields(url = %query.url))]
pub async fn stream_audio(
    State(state):      State<AppState>,
    AuthUser(_claims): AuthUser,
    Query(query):      Query<StreamQuery>,
) -> Result<Response, AppError> {
    // ── 1. Normalise input URL ────────────────────────────────────────────────
    let canonical_url = normalise_youtube_url(&query.url).map_err(AppError::from_http)?;

    info!(%canonical_url, "fetching video info");

    // ── 2. Fetch video metadata via yt-dlp ───────────────────────────────────
    let video = state
        .downloader
        .fetch_video_infos(&canonical_url)
        .await
        .map_err(|e| AppError::internal(format!("yt-dlp fetch failed: {e}")))?;

    // ── 3. Select best audio-only format ─────────────────────────────────────
    let format = video.best_audio_format().ok_or_else(|| {
        AppError::from_http(HttpError::unprocessable(
            "no audio-only format available for this video",
        ))
    })?;

    // ── 4. Extract CDN URL ───────────────────────────────────────────────────
    let cdn_url = format
        .url()
        .map_err(|e| AppError::internal(format!("format has no URL: {e}")))?
        .clone();

    let mime     = audio_mime(&format);
    let filesize = format.file_info.filesize
        .or(format.file_info.filesize_approx);

    // Forward the YouTube-required headers so the CDN doesn't reject us.
    let yt_headers = format.download_info.http_headers.to_header_map();

    info!(
        video_id  = %video.id,
        title     = %video.title,
        mime      = %mime,
        filesize  = ?filesize,
        "streaming audio format"
    );

    // ── 5. Build response headers ─────────────────────────────────────────────
    let mut resp_headers = HeaderMap::new();

    resp_headers.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_str(&mime)
            .unwrap_or(HeaderValue::from_static("application/octet-stream")),
    );

    let safe_title  = sanitise_filename(&video.title);
    let ext         = ext_for_mime(&mime);
    // RFC 5987: encode non-ASCII filename as UTF-8 percent-encoded.
    // This lets browsers correctly display Unicode titles (e.g. Korean, Japanese).
    let pct_title   = percent_encode_filename(&format!("{safe_title}.{ext}"));
    let disposition = format!("inline; filename*=UTF-8''{pct_title}");
    if let Ok(v) = HeaderValue::from_str(&disposition) {
        resp_headers.insert(header::CONTENT_DISPOSITION, v);
    }

    if let Some(size) = filesize {
        if let Ok(v) = HeaderValue::from_str(&size.to_string()) {
            resp_headers.insert(header::CONTENT_LENGTH, v);
        }
    }

    resp_headers.insert(header::ACCEPT_RANGES, HeaderValue::from_static("bytes"));

    // ── 6. Choose download strategy ──────────────────────────────────────────
    //
    // If we know the file size, split it into PARALLEL_CHUNKS Range requests
    // that run concurrently.  Each YouTube CDN connection is throttled to
    // ~32 KB/s, but N connections give us N × 32 KB/s aggregate throughput.
    //
    // If the file size is unknown, fall back to a single streaming request.

    let body = match filesize.filter(|&s| s >= MIN_CHUNK_SIZE as i64) {
        Some(total_bytes) => {
            parallel_range_body(
                state.http_client.clone(),
                cdn_url,
                yt_headers,
                total_bytes as u64,
                parallel_chunks(),
            )
            .await?
        }
        None => {
            // Fallback: single streaming request
            let upstream = state
                .http_client
                .get(&cdn_url)
                .headers(yt_headers)
                .send()
                .await
                .map_err(|e| AppError::internal(format!("upstream CDN request failed: {e}")))?;

            if !upstream.status().is_success() {
                return Err(AppError::from_http(HttpError::new(
                    StatusCode::BAD_GATEWAY,
                    "BAD_GATEWAY",
                    format!("CDN returned HTTP {}", upstream.status()),
                )));
            }

            Body::from_stream(upstream.bytes_stream())
        }
    };

    Ok((StatusCode::OK, resp_headers, body).into_response())
}

/// Downloads `total_bytes` from `url` using `PARALLEL_CHUNKS` concurrent
/// Range requests, then streams the assembled bytes in order to the caller.
///
/// The actual assembly happens asynchronously via a channel: each chunk task
/// writes its bytes to the channel in the correct slot order, and a background
/// task reads them sequentially and forwards them to the response body stream.
async fn parallel_range_body(
    client:      reqwest::Client,
    url:         String,
    headers:     HeaderMap,
    total_bytes: u64,
    num_chunks:  u64,
) -> Result<Body, AppError> {
    let chunk_size = (total_bytes + num_chunks - 1) / num_chunks;

    // Build (start, end) byte ranges — end is inclusive per HTTP spec.
    let ranges: Vec<(u64, u64)> = (0..num_chunks)
        .map(|i| {
            let start = i * chunk_size;
            let end   = ((i + 1) * chunk_size - 1).min(total_bytes - 1);
            (start, end)
        })
        .filter(|(s, e)| s <= e)
        .collect();

    let n = ranges.len();
    info!(chunks = n, total_bytes, "starting parallel Range download");

    // Fetch all chunks concurrently.
    let futures: Vec<_> = ranges
        .into_iter()
        .enumerate()
        .map(|(idx, (start, end))| {
            let client  = client.clone();
            let url     = url.clone();
            let headers = headers.clone();
            async move {
                let range_val = format!("bytes={start}-{end}");
                let resp = client
                    .get(&url)
                    .headers(headers)
                    .header(header::RANGE, &range_val)
                    .send()
                    .await
                    .map_err(|e| format!("chunk {idx} request failed: {e}"))?;

                if !resp.status().is_success() && resp.status() != StatusCode::PARTIAL_CONTENT {
                    return Err(format!(
                        "chunk {idx} CDN returned HTTP {}",
                        resp.status()
                    ));
                }

                let bytes = resp
                    .bytes()
                    .await
                    .map_err(|e| format!("chunk {idx} read failed: {e}"))?;

                info!(chunk = idx, bytes = bytes.len(), "chunk downloaded");
                Ok::<_, String>((idx, bytes))
            }
        })
        .collect();

    let results = future::join_all(futures).await;

    // Collect chunks in order, propagating any error.
    let mut chunks: Vec<Option<Bytes>> = (0..n).map(|_| None).collect();
    for result in results {
        match result {
            Ok((idx, data)) => chunks[idx] = Some(data),
            Err(e) => {
                return Err(AppError::internal(format!("parallel download failed: {e}")));
            }
        }
    }

    // Stream all chunks sequentially through a channel-backed Body.
    let (tx, rx) = mpsc::channel::<Result<Bytes, std::io::Error>>(n);

    tokio::spawn(async move {
        for chunk in chunks.into_iter().flatten() {
            if tx.send(Ok(chunk)).await.is_err() {
                break; // client disconnected
            }
        }
    });

    Ok(Body::from_stream(ReceiverStream::new(rx)))
}

// ─── helpers ─────────────────────────────────────────────────────────────────

/// Maps a yt-dlp [`Extension`] enum to a `Content-Type` MIME string.
fn audio_mime(format: &Format) -> String {
    match format.download_info.ext {
        Extension::M4A  => "audio/mp4".to_owned(),
        Extension::Mp3  => "audio/mpeg".to_owned(),
        Extension::Ogg  => "audio/ogg".to_owned(),
        Extension::Flac => "audio/flac".to_owned(),
        Extension::Wav  => "audio/wav".to_owned(),
        Extension::Webm => "audio/webm".to_owned(),
        Extension::Aac  => "audio/aac".to_owned(),
        _ => {
            let ext = format.download_info.ext.as_str();
            format!("audio/{ext}")
        }
    }
}

/// Maps a MIME string back to a short file extension for Content-Disposition.
fn ext_for_mime(mime: &str) -> &str {
    if mime.starts_with("audio/mp4")   { return "m4a"; }
    if mime.starts_with("audio/mpeg")  { return "mp3"; }
    if mime.starts_with("audio/ogg")   { return "ogg"; }
    if mime.starts_with("audio/flac")  { return "flac"; }
    if mime.starts_with("audio/wav")   { return "wav"; }
    if mime.starts_with("audio/webm")  { return "webm"; }
    if mime.starts_with("audio/aac")   { return "aac"; }
    "bin"
}

/// Replaces characters that are unsafe in filenames with an underscore.
fn sanitise_filename(title: &str) -> String {
    title
        .chars()
        .map(|c| match c {
            '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' => '_',
            c => c,
        })
        .take(200)
        .collect()
}

/// Percent-encodes a filename for use in a RFC 5987 `filename*=UTF-8''...`
/// Content-Disposition value.  Only unreserved URI characters are left as-is;
/// everything else (including non-ASCII / Unicode) is %-encoded.
fn percent_encode_filename(name: &str) -> String {
    name.bytes()
        .flat_map(|b| {
            if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~') {
                vec![b as char]
            } else {
                format!("%{b:02X}").chars().collect::<Vec<_>>()
            }
        })
        .collect()
}
