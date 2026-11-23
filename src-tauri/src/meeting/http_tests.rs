//! The endpoint over real HTTP, with a folder standing in for History.

use std::io::{Read, Write};
use std::net::TcpStream;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};

use anyhow::Result;
use hound::WavReader;
use serde_json::Value;
use tiny_http::Server;

use super::server::{self, Shared};
use super::session::{self, MeetingStore, Session};

const TOKEN: &str = "test-token";

type Added = Arc<Mutex<Vec<(String, String)>>>;

struct FolderStore {
    dir: PathBuf,
    added: Added,
}

impl MeetingStore for FolderStore {
    fn recordings_dir(&self) -> PathBuf {
        self.dir.clone()
    }

    fn add(&self, file_name: String, title: String) -> Result<i64> {
        let mut added = self.added.lock().unwrap();
        added.push((file_name, title));
        Ok(added.len() as i64)
    }
}

/// A running endpoint on a free port, stopped when dropped.
struct Endpoint {
    port: u16,
    dir: tempfile::TempDir,
    added: Added,
    server: Arc<Server>,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl Endpoint {
    fn start() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let added = Added::default();
        let shared = Arc::new(Shared {
            store: Box::new(FolderStore {
                dir: dir.path().to_path_buf(),
                added: Arc::clone(&added),
            }),
            token: Mutex::new(TOKEN.to_string()),
            session: Mutex::new(None),
        });
        let server = Arc::new(Server::http("127.0.0.1:0").unwrap());
        let port = server.server_addr().to_ip().unwrap().port();
        let stop = Arc::new(AtomicBool::new(false));
        let thread = thread::spawn({
            let (server, stop) = (Arc::clone(&server), Arc::clone(&stop));
            move || server::run(&server, &shared, &stop)
        });
        Self {
            port,
            dir,
            added,
            server,
            stop,
            thread: Some(thread),
        }
    }

    /// A request as the extension sends it: with the token and its origin.
    fn send(&self, method: &str, path: &str, body: &[u8]) -> (u16, Value) {
        let authorization = format!("Bearer {TOKEN}");
        self.send_with(
            method,
            path,
            &[
                ("Authorization", &authorization),
                (
                    "Origin",
                    "chrome-extension://abcdefghijklmnopabcdefghijklmnop",
                ),
            ],
            body,
        )
    }

    /// A request with exactly these headers, plus `Host` unless given.
    fn send_with(
        &self,
        method: &str,
        path: &str,
        headers: &[(&str, &str)],
        body: &[u8],
    ) -> (u16, Value) {
        let mut stream = TcpStream::connect(("127.0.0.1", self.port)).unwrap();
        let mut head = format!(
            "{method} {path} HTTP/1.1\r\nConnection: close\r\nContent-Length: {}\r\n",
            body.len()
        );
        if !headers
            .iter()
            .any(|(name, _)| name.eq_ignore_ascii_case("host"))
        {
            head += &format!("Host: 127.0.0.1:{}\r\n", self.port);
        }
        for (name, value) in headers {
            head += &format!("{name}: {value}\r\n");
        }
        head += "\r\n";
        stream.write_all(head.as_bytes()).unwrap();
        stream.write_all(body).unwrap();

        let mut response = Vec::new();
        stream.read_to_end(&mut response).unwrap();
        let response = String::from_utf8_lossy(&response);
        let status = response
            .split_whitespace()
            .nth(1)
            .and_then(|status| status.parse().ok())
            .expect("a status line");
        let body = response.split_once("\r\n\r\n").map_or("", |(_, body)| body);
        (status, serde_json::from_str(body).unwrap_or(Value::Null))
    }

    fn start_meeting(&self, title: &str) -> String {
        let body = serde_json::json!({ "title": title }).to_string();
        let (status, reply) = self.send("POST", "/v1/meetings", body.as_bytes());
        assert_eq!(status, 201, "{reply}");
        reply["id"].as_str().unwrap().to_string()
    }

    fn files(&self) -> Vec<String> {
        let mut files: Vec<String> = std::fs::read_dir(self.dir.path())
            .unwrap()
            .map(|file| file.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        files.sort();
        files
    }
}

impl Drop for Endpoint {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        self.server.unblock();
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

/// `frames` of audio from `first`: the frame number on the left, its negative
/// on the right, so every sample says where it belongs.
fn pcm(first: usize, frames: usize) -> Vec<u8> {
    (first..first + frames)
        .flat_map(|frame| {
            let left = (frame % 30_000) as i16;
            [left.to_le_bytes(), (-left).to_le_bytes()].concat()
        })
        .collect()
}

#[test]
fn a_meeting_becomes_a_two_channel_recording() {
    let endpoint = Endpoint::start();
    let id = endpoint.start_meeting("Weekly sync - Meet");
    let audio = format!("/v1/meetings/{id}/audio");

    // 2.5 s in three pieces, in order.
    for (seq, first, frames) in [(0, 0, 16_000), (1, 16_000, 16_000), (2, 32_000, 8_000)] {
        let (status, reply) =
            endpoint.send("POST", &format!("{audio}?seq={seq}"), &pcm(first, frames));
        assert_eq!(status, 200, "{reply}");
        assert_eq!(reply["next_seq"], seq + 1);
    }
    let (_, status) = endpoint.send("GET", "/v1/status", b"");
    assert_eq!(status["meeting"]["id"], id.as_str());
    assert_eq!(status["meeting"]["seconds"], 2.5);
    assert_eq!(endpoint.files().len(), 1, "one partial recording so far");
    assert!(endpoint.files()[0].ends_with(".wav.partial"));

    let (status, reply) = endpoint.send("POST", &format!("/v1/meetings/{id}/finish"), b"");
    assert_eq!(status, 200, "{reply}");
    assert_eq!(reply["entry_id"], 1);
    assert_eq!(reply["seconds"], 2.5);

    // It went to the store under its real name, with its title.
    let added = endpoint.added.lock().unwrap().clone();
    assert_eq!(added.len(), 1);
    assert_eq!(added[0].1, "Weekly sync - Meet");
    assert_eq!(endpoint.files(), vec![added[0].0.clone()]);

    // Two channels at 16 kHz, every sample where it was sent.
    let reader = WavReader::open(endpoint.dir.path().join(&added[0].0)).unwrap();
    let spec = reader.spec();
    assert_eq!(
        (spec.channels, spec.sample_rate, spec.bits_per_sample),
        (2, 16_000, 16)
    );
    let samples: Vec<i16> = reader.into_samples().map(Result::unwrap).collect();
    assert_eq!(samples.len(), 40_000 * 2);
    for frame in [0usize, 1, 15_999, 16_000, 39_999] {
        assert_eq!(
            samples[frame * 2],
            (frame % 30_000) as i16,
            "left of {frame}"
        );
        assert_eq!(
            samples[frame * 2 + 1],
            -((frame % 30_000) as i16),
            "right of {frame}"
        );
    }

    // The meeting is over: nothing more is taken for it.
    let (status, _) = endpoint.send("POST", &format!("{audio}?seq=3"), &pcm(0, 16));
    assert_eq!(status, 404);
    let (_, status) = endpoint.send("GET", "/v1/status", b"");
    assert!(status["meeting"].is_null());
}

#[test]
fn audio_is_written_once_and_in_order() {
    let endpoint = Endpoint::start();
    let id = endpoint.start_meeting("Order");
    let audio = format!("/v1/meetings/{id}/audio");

    let (status, _) = endpoint.send("POST", &format!("{audio}?seq=0"), &pcm(0, 16_000));
    assert_eq!(status, 200);
    // The same piece again, as after a lost answer: accepted, not written.
    let (status, reply) = endpoint.send("POST", &format!("{audio}?seq=0"), &pcm(0, 16_000));
    assert_eq!((status, &reply["next_seq"]), (200, &Value::from(1)));
    assert_eq!(reply["seconds"], 1.0);
    // A piece from further on: turned away, with what is expected.
    let (status, reply) = endpoint.send("POST", &format!("{audio}?seq=4"), &pcm(0, 16_000));
    assert_eq!(status, 409);
    assert_eq!(reply["error"], "out_of_order");
    assert_eq!(reply["next_seq"], 1);
    // Half a frame, no audio, and no number.
    let (status, reply) = endpoint.send("POST", &format!("{audio}?seq=1"), &[0, 0, 0]);
    assert_eq!((status, reply["error"].as_str()), (400, Some("bad_audio")));
    let (status, _) = endpoint.send("POST", &format!("{audio}?seq=1"), b"");
    assert_eq!(status, 400);
    let (status, reply) = endpoint.send("POST", &audio, &pcm(0, 16));
    assert_eq!(
        (status, reply["error"].as_str()),
        (400, Some("missing_seq"))
    );

    // One meeting at a time while it is active.
    let (status, reply) = endpoint.send("POST", "/v1/meetings", b"{}");
    assert_eq!(
        (status, reply["error"].as_str()),
        (409, Some("meeting_in_progress"))
    );

    let (status, reply) = endpoint.send("POST", &format!("/v1/meetings/{id}/finish"), b"");
    assert_eq!(status, 200, "{reply}");
    assert_eq!(reply["seconds"], 1.0);
}

#[test]
fn only_a_paired_extension_gets_in() {
    let endpoint = Endpoint::start();
    let bearer = format!("Bearer {TOKEN}");
    let extension = "chrome-extension://abcdefghijklmnopabcdefghijklmnop";

    // No token, a wrong one, and the right one in the wrong form.
    for headers in [
        vec![("Origin", extension)],
        vec![("Authorization", "Bearer wrong"), ("Origin", extension)],
        vec![("Authorization", TOKEN), ("Origin", extension)],
    ] {
        let (status, reply) = endpoint.send_with("GET", "/v1/status", &headers, b"");
        assert_eq!(
            (status, reply["error"].as_str()),
            (401, Some("unauthorized"))
        );
    }
    // A web page, even with the token.
    let (status, reply) = endpoint.send_with(
        "POST",
        "/v1/meetings",
        &[
            ("Authorization", &bearer),
            ("Origin", "https://evil.example"),
        ],
        b"{}",
    );
    assert_eq!(
        (status, reply["error"].as_str()),
        (403, Some("forbidden_origin"))
    );
    // A host name pointed at this computer, even with the token.
    let (status, _) = endpoint.send_with(
        "GET",
        "/v1/status",
        &[("Authorization", &bearer), ("Host", "evil.example")],
        b"",
    );
    assert_eq!(status, 403);
    // Nothing was started by any of them.
    assert!(endpoint.files().is_empty());

    // The extension, and a preflight for it, which carries no token.
    let (status, reply) = endpoint.send("GET", "/v1/status", b"");
    assert_eq!((status, reply["app"].as_str()), (200, Some("whispr")));
    let (status, _) = endpoint.send_with("OPTIONS", "/v1/meetings", &[("Origin", extension)], b"");
    assert_eq!(status, 200);
    let (status, _) = endpoint.send("GET", "/v1/elsewhere", b"");
    assert_eq!(status, 404);
}

#[test]
fn a_meeting_without_audio_or_discarded_leaves_nothing() {
    let endpoint = Endpoint::start();

    // Under a second: not kept.
    let id = endpoint.start_meeting("Too short");
    let (status, _) = endpoint.send(
        "POST",
        &format!("/v1/meetings/{id}/audio?seq=0"),
        &pcm(0, 8_000),
    );
    assert_eq!(status, 200);
    let (status, reply) = endpoint.send("POST", &format!("/v1/meetings/{id}/finish"), b"");
    assert_eq!((status, reply["error"].as_str()), (422, Some("not_saved")));
    assert!(endpoint.files().is_empty());

    // Discarded.
    let id = endpoint.start_meeting("Discarded");
    let (status, _) = endpoint.send(
        "POST",
        &format!("/v1/meetings/{id}/audio?seq=0"),
        &pcm(0, 32_000),
    );
    assert_eq!(status, 200);
    let (status, _) = endpoint.send("DELETE", &format!("/v1/meetings/{id}"), b"");
    assert_eq!(status, 200);
    assert!(endpoint.files().is_empty());
    assert!(endpoint.added.lock().unwrap().is_empty());

    // Neither is there to finish any more.
    let (status, reply) = endpoint.send("POST", &format!("/v1/meetings/{id}/finish"), b"");
    assert_eq!(
        (status, reply["error"].as_str()),
        (404, Some("unknown_meeting"))
    );
}

#[test]
fn turning_capture_off_keeps_the_meeting_in_progress() {
    let mut endpoint = Endpoint::start();
    let id = endpoint.start_meeting("Interrupted");
    let (status, _) = endpoint.send(
        "POST",
        &format!("/v1/meetings/{id}/audio?seq=0"),
        &pcm(0, 32_000),
    );
    assert_eq!(status, 200);

    // What turning the setting off does: the thread finishes the meeting on
    // its way out.
    endpoint.stop.store(true, Ordering::Release);
    endpoint.server.unblock();
    endpoint.thread.take().unwrap().join().unwrap();

    let added = endpoint.added.lock().unwrap().clone();
    assert_eq!(added.len(), 1);
    assert_eq!(added[0].1, "Interrupted");
    assert_eq!(endpoint.files(), vec![added[0].0.clone()]);
}

#[test]
fn a_meeting_cut_short_by_a_crash_is_recovered() {
    let dir = tempfile::tempdir().unwrap();
    let added = Added::default();
    let store = FolderStore {
        dir: dir.path().to_path_buf(),
        added: Arc::clone(&added),
    };

    let mut interrupted = Session::start(&store, "a".into(), "Lost title").unwrap();
    interrupted.append(&pcm(0, 48_000)).unwrap();
    // Handy stops here: the recording is never closed.
    std::mem::forget(interrupted);
    // One that never got a second of audio is not worth keeping.
    let mut empty = Session::start(&store, "b".into(), "Empty").unwrap();
    empty.append(&pcm(0, 160)).unwrap();
    std::mem::forget(empty);

    session::recover_unfinished(&store);

    let added = added.lock().unwrap().clone();
    assert_eq!(added.len(), 1, "{added:?}");
    assert_eq!(added[0].1, "Meeting (recovered)");
    let reader = WavReader::open(dir.path().join(&added[0].0)).unwrap();
    assert_eq!(reader.spec().channels, 2);
    assert_eq!(reader.duration(), 48_000);
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
}
