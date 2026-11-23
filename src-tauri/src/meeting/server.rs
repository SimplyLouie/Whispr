//! The loopback HTTP endpoint the browser extension sends a meeting to.
//!
//! Every request carries `Authorization: Bearer <pairing token>`. A request
//! with an `Origin` is only accepted from a browser extension, so a web page
//! can't reach the endpoint even if it learns the token.
//!
//! | Request                              | Does                                   |
//! | ------------------------------------ | -------------------------------------- |
//! | `GET /v1/status`                     | Says Handy is here, and what it records |
//! | `POST /v1/meetings` `{"title"}`      | Starts a meeting, returns its `id`     |
//! | `POST /v1/meetings/{id}/audio?seq=N` | Adds audio, in order from `seq=0`      |
//! | `POST /v1/meetings/{id}/finish`      | Ends it and queues its transcription   |
//! | `DELETE /v1/meetings/{id}`           | Drops it                               |
//!
//! Audio is raw 16 kHz, 16-bit little-endian, two channels interleaved: the
//! microphone on the left, the meeting tab on the right.

use std::io::Read;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, MutexGuard};
use std::time::Duration;

use log::{debug, error, info, warn};
use serde::Deserialize;
use serde_json::{json, Value};
use tiny_http::{Header, Method, Request, Response, Server};

use super::session::{MeetingStore, Session, FRAME_BYTES, MAX_FRAMES};

/// How often the loop looks at the stop flag and at an idle meeting.
const POLL: Duration = Duration::from_secs(1);
/// A meeting that got no audio for this long has lost its extension (the
/// browser quit, the computer slept); it is finished with what it has.
const IDLE_TIMEOUT: Duration = Duration::from_secs(45);
/// A new meeting may replace one that has been quiet this long.
const TAKEOVER_AFTER: Duration = Duration::from_secs(10);
/// The extension sends a few seconds at a time, a minute after a retry.
const MAX_AUDIO_BYTES: usize = 16 * 1024 * 1024;
const MAX_JSON_BYTES: usize = 64 * 1024;

/// What the endpoint and the manager both use.
pub(super) struct Shared {
    pub(super) store: Box<dyn MeetingStore>,
    pub(super) token: Mutex<String>,
    pub(super) session: Mutex<Option<Session>>,
}

impl Shared {
    pub(super) fn session(&self) -> MutexGuard<'_, Option<Session>> {
        self.session
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    pub(super) fn token(&self) -> MutexGuard<'_, String> {
        self.token
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// Ends the meeting being recorded, if any, keeping its audio.
    fn finish_session(&self, session: Session, why: &str) {
        let seconds = session.seconds();
        match session.finish(self.store.as_ref()) {
            Ok(_) => info!("Finished a {seconds:.0}s meeting: {why}"),
            Err(e) => warn!("A {seconds:.0}s meeting was not saved ({why}): {e:#}"),
        }
    }
}

/// Serves requests until `stop` is set, then finishes any meeting in progress.
pub(super) fn run(server: &Server, shared: &Shared, stop: &AtomicBool) {
    while !stop.load(Ordering::Acquire) {
        match server.recv_timeout(POLL) {
            Ok(Some(request)) => handle(shared, request),
            Ok(None) => {}
            Err(e) => {
                error!("Meeting capture stopped listening: {e}");
                break;
            }
        }
        let idle = {
            let mut session = shared.session();
            match session.as_ref() {
                Some(active) if active.idle_for() > IDLE_TIMEOUT => session.take(),
                _ => None,
            }
        };
        if let Some(session) = idle {
            shared.finish_session(session, "no audio arrived any more");
        }
    }
    let last = shared.session().take();
    if let Some(session) = last {
        shared.finish_session(session, "meeting capture was turned off");
    }
}

struct Reply {
    status: u16,
    body: Value,
}

impl Reply {
    fn ok(body: Value) -> Self {
        Self { status: 200, body }
    }

    fn error(status: u16, code: &str) -> Self {
        Self {
            status,
            body: json!({ "error": code }),
        }
    }

    fn error_with_detail(status: u16, code: &str, detail: impl ToString) -> Self {
        Self {
            status,
            body: json!({ "error": code, "detail": detail.to_string() }),
        }
    }
}

fn handle(shared: &Shared, mut request: Request) {
    let origin = header(&request, "Origin").map(str::to_owned);
    let reply = route(shared, &mut request, origin.as_deref());

    let mut response = Response::from_string(reply.body.to_string()).with_status_code(reply.status);
    let mut headers = vec![("Content-Type", "application/json")];
    // An extension with host access needs none of these, but they keep a
    // preflight working if the browser sends one.
    if let Some(origin) = origin.as_deref().filter(|origin| is_extension(origin)) {
        headers.extend([
            ("Access-Control-Allow-Origin", origin),
            ("Access-Control-Allow-Methods", "GET, POST, DELETE, OPTIONS"),
            (
                "Access-Control-Allow-Headers",
                "Authorization, Content-Type",
            ),
            ("Access-Control-Allow-Private-Network", "true"),
            ("Vary", "Origin"),
        ]);
    }
    for (name, value) in headers {
        if let Ok(header) = Header::from_bytes(name, value) {
            response.add_header(header);
        }
    }
    if let Err(e) = request.respond(response) {
        debug!("Failed to answer a meeting capture request: {e}");
    }
}

fn route(shared: &Shared, request: &mut Request, origin: Option<&str>) -> Reply {
    if !request
        .remote_addr()
        .is_some_and(|address| address.ip().is_loopback())
    {
        return Reply::error(403, "forbidden");
    }
    // A page that points its own host name at this computer still sends that
    // name, and its own origin.
    if !header(request, "Host").is_some_and(is_loopback_host) {
        return Reply::error(403, "forbidden");
    }
    if origin.is_some_and(|origin| !is_extension(origin)) {
        return Reply::error(403, "forbidden_origin");
    }
    let method = request.method().clone();
    if method == Method::Options {
        return Reply::ok(json!({}));
    }
    if !authorized(shared, request) {
        return Reply::error(401, "unauthorized");
    }

    let url = request.url().to_owned();
    let (path, query) = url.split_once('?').unwrap_or((&url, ""));
    let segments: Vec<&str> = path.trim_matches('/').split('/').collect();
    match (&method, segments.as_slice()) {
        (Method::Get, ["v1", "status"]) => status(shared),
        (Method::Post, ["v1", "meetings"]) => start(shared, request),
        (Method::Post, ["v1", "meetings", id, "audio"]) => audio(shared, request, id, query),
        (Method::Post, ["v1", "meetings", id, "finish"]) => finish(shared, id),
        (Method::Delete, ["v1", "meetings", id]) => discard(shared, id),
        _ => Reply::error(404, "not_found"),
    }
}

fn status(shared: &Shared) -> Reply {
    let meeting = shared
        .session()
        .as_ref()
        .map(|session| json!({ "id": session.id, "seconds": session.seconds() }));
    Reply::ok(json!({
        "app": "whispr",
        "version": env!("CARGO_PKG_VERSION"),
        "meeting": meeting,
    }))
}

#[derive(Deserialize, Default)]
struct StartRequest {
    #[serde(default)]
    title: String,
}

fn start(shared: &Shared, request: &mut Request) -> Reply {
    let body = match read_body(request, MAX_JSON_BYTES) {
        Ok(body) => body,
        Err(reply) => return reply,
    };
    let start: StartRequest = if body.is_empty() {
        StartRequest::default()
    } else {
        match serde_json::from_slice(&body) {
            Ok(start) => start,
            Err(e) => return Reply::error_with_detail(400, "bad_request", e),
        }
    };

    let mut session = shared.session();
    if let Some(active) = session.as_ref() {
        if active.idle_for() < TAKEOVER_AFTER {
            return Reply::error(409, "meeting_in_progress");
        }
        // Whoever was sending it has stopped; keep what it recorded.
        if let Some(abandoned) = session.take() {
            shared.finish_session(abandoned, "another meeting started");
        }
    }
    let id = match super::random_hex(16) {
        Ok(id) => id,
        Err(e) => return Reply::error_with_detail(500, "not_started", e),
    };
    match Session::start(shared.store.as_ref(), id.clone(), &start.title) {
        Ok(started) => {
            info!("Started recording a meeting");
            *session = Some(started);
            Reply {
                status: 201,
                body: json!({ "id": id }),
            }
        }
        Err(e) => {
            error!("Failed to start a meeting recording: {e:#}");
            Reply::error_with_detail(500, "not_started", format!("{e:#}"))
        }
    }
}

fn audio(shared: &Shared, request: &mut Request, id: &str, query: &str) -> Reply {
    let Some(seq) = query_value(query, "seq").and_then(|seq| seq.parse::<u64>().ok()) else {
        return Reply::error(400, "missing_seq");
    };
    // Read before taking the lock: a slow sender must not hold it.
    let pcm = match read_body(request, MAX_AUDIO_BYTES) {
        Ok(pcm) => pcm,
        Err(reply) => return reply,
    };
    if pcm.is_empty() || !pcm.len().is_multiple_of(FRAME_BYTES) {
        return Reply::error(400, "bad_audio");
    }

    let mut guard = shared.session();
    let Some(session) = guard.as_mut().filter(|session| session.id == id) else {
        return Reply::error(404, "unknown_meeting");
    };
    let progress =
        |session: &Session| json!({ "next_seq": session.next_seq, "seconds": session.seconds() });
    if seq < session.next_seq {
        // Sent again after an answer got lost; it is already written.
        return Reply::ok(progress(session));
    }
    if seq > session.next_seq {
        return Reply {
            status: 409,
            body: json!({ "error": "out_of_order", "next_seq": session.next_seq }),
        };
    }
    if session.frames() + (pcm.len() / FRAME_BYTES) as u64 > MAX_FRAMES {
        if let Some(full) = guard.take() {
            shared.finish_session(full, "it reached the longest meeting Handy records");
        }
        return Reply::error(409, "meeting_too_long");
    }
    match session.append(&pcm) {
        Ok(()) => Reply::ok(progress(session)),
        Err(e) => {
            error!("Failed to write meeting audio: {e:#}");
            Reply::error_with_detail(500, "write_failed", format!("{e:#}"))
        }
    }
}

fn finish(shared: &Shared, id: &str) -> Reply {
    let Some(session) = take_session(shared, id) else {
        return Reply::error(404, "unknown_meeting");
    };
    let seconds = session.seconds();
    match session.finish(shared.store.as_ref()) {
        Ok(entry_id) => Reply::ok(json!({ "entry_id": entry_id, "seconds": seconds })),
        Err(e) => {
            warn!("A {seconds:.0}s meeting was not saved: {e:#}");
            Reply::error_with_detail(422, "not_saved", format!("{e:#}"))
        }
    }
}

fn discard(shared: &Shared, id: &str) -> Reply {
    let Some(session) = take_session(shared, id) else {
        return Reply::error(404, "unknown_meeting");
    };
    session.discard();
    info!("Discarded a meeting recording");
    Reply::ok(json!({}))
}

/// The meeting in progress, if it is `id`.
fn take_session(shared: &Shared, id: &str) -> Option<Session> {
    let mut session = shared.session();
    if session.as_ref().is_some_and(|session| session.id == id) {
        session.take()
    } else {
        None
    }
}

/// The request's body, or the reply that turns it down for being over `limit`.
fn read_body(request: &mut Request, limit: usize) -> Result<Vec<u8>, Reply> {
    if request.body_length().is_some_and(|length| length > limit) {
        return Err(Reply::error(413, "too_large"));
    }
    let mut body = Vec::new();
    request
        .as_reader()
        .take(limit as u64 + 1)
        .read_to_end(&mut body)
        .map_err(|e| Reply::error_with_detail(400, "bad_request", e))?;
    if body.len() > limit {
        return Err(Reply::error(413, "too_large"));
    }
    Ok(body)
}

fn header<'a>(request: &'a Request, name: &'static str) -> Option<&'a str> {
    request
        .headers()
        .iter()
        .find(|header| header.field.equiv(name))
        .map(|header| header.value.as_str())
}

fn authorized(shared: &Shared, request: &Request) -> bool {
    let Some(presented) = header(request, "Authorization")
        .and_then(|value| value.strip_prefix("Bearer "))
        .map(str::trim)
    else {
        return false;
    };
    let token = shared.token();
    !token.is_empty() && same_bytes(presented.as_bytes(), token.as_bytes())
}

/// Equality that takes the same time wherever the first difference is.
fn same_bytes(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |diff, (a, b)| diff | (a ^ b)) == 0
}

fn is_extension(origin: &str) -> bool {
    origin.starts_with("chrome-extension://")
}

/// Whether a `Host` header names this computer, with or without a port.
fn is_loopback_host(host: &str) -> bool {
    let name = match host.rsplit_once(':') {
        Some((name, port)) if !port.is_empty() && port.bytes().all(|b| b.is_ascii_digit()) => name,
        _ => host,
    };
    matches!(name, "127.0.0.1" | "localhost" | "[::1]")
}

fn query_value<'a>(query: &'a str, name: &str) -> Option<&'a str> {
    query
        .split('&')
        .filter_map(|pair| pair.split_once('='))
        .find(|(key, _)| *key == name)
        .map(|(_, value)| value)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_this_computer_is_a_loopback_host() {
        for host in [
            "127.0.0.1:47821",
            "127.0.0.1",
            "localhost:47821",
            "[::1]:47821",
        ] {
            assert!(is_loopback_host(host), "{host}");
        }
        for host in [
            "evil.example:47821",
            "127.0.0.1.evil.example",
            "localhost.evil.example:80",
            "",
        ] {
            assert!(!is_loopback_host(host), "{host}");
        }
    }

    #[test]
    fn only_extensions_are_accepted_origins() {
        assert!(is_extension("chrome-extension://abcdefghijklmnop"));
        assert!(!is_extension("https://meet.google.com"));
        assert!(!is_extension("http://127.0.0.1:47821"));
    }

    #[test]
    fn tokens_compare_whole() {
        assert!(same_bytes(b"abc123", b"abc123"));
        assert!(!same_bytes(b"abc123", b"abc124"));
        assert!(!same_bytes(b"abc", b"abc123"));
        assert!(same_bytes(b"", b""));
    }

    #[test]
    fn query_values_are_found_by_name() {
        assert_eq!(query_value("seq=12", "seq"), Some("12"));
        assert_eq!(query_value("a=1&seq=3&b=2", "seq"), Some("3"));
        assert_eq!(query_value("sequence=9", "seq"), None);
        assert_eq!(query_value("", "seq"), None);
    }
}
