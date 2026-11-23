//! Meeting capture: a browser extension records a meeting tab and the
//! microphone, and streams both to Handy over an HTTP endpoint that only
//! listens on this computer. When the meeting ends it goes to History and is
//! transcribed with the speakers told apart (see [`transcript`]).
//!
//! The endpoint is off until the user turns it on, and every request has to
//! carry the pairing token the settings show (see [`server`]).

#[cfg(test)]
mod http_tests;
mod server;
mod session;
pub mod transcript;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread::{self, JoinHandle};

use anyhow::{anyhow, Result};
use log::{info, warn};
use serde::{Deserialize, Serialize};
use specta::Type;
use tauri::AppHandle;
use tiny_http::Server;

use crate::settings::{get_settings, write_settings};
use server::Shared;
use session::HistoryStore;

/// The port the endpoint listens on, on 127.0.0.1 only. The extension uses
/// the same one by default.
pub const PORT: u16 = 47_821;

/// What the settings show about meeting capture.
#[derive(Clone, Debug, Serialize, Deserialize, Type)]
pub struct MeetingCaptureStatus {
    pub enabled: bool,
    /// Whether the endpoint is accepting requests.
    pub listening: bool,
    pub port: u16,
    /// The pairing token to enter in the extension; empty before meeting
    /// capture was ever turned on.
    pub token: String,
    /// Why the endpoint isn't listening although it is enabled.
    pub error: Option<String>,
    /// How much of the meeting in progress is recorded, if there is one.
    pub recording_seconds: Option<u32>,
}

/// The endpoint while it runs.
struct Running {
    server: Arc<Server>,
    stop: Arc<AtomicBool>,
    thread: JoinHandle<()>,
}

#[derive(Default)]
struct Listener {
    running: Option<Running>,
    /// Why the last start failed.
    error: Option<String>,
}

pub struct MeetingCaptureManager {
    app: AppHandle,
    shared: Arc<Shared>,
    listener: Mutex<Listener>,
}

impl MeetingCaptureManager {
    /// Sends meetings left unfinished by the last run to History, and starts
    /// the endpoint if meeting capture is on. History and the transcription
    /// jobs must already be managed.
    pub fn new(app: &AppHandle) -> Arc<Self> {
        let settings = get_settings(app);
        let manager = Arc::new(Self {
            app: app.clone(),
            shared: Arc::new(Shared {
                store: Box::new(HistoryStore(app.clone())),
                token: Mutex::new(settings.meeting_capture_token),
                session: Mutex::new(None),
            }),
            listener: Mutex::default(),
        });
        session::recover_unfinished(manager.shared.store.as_ref());
        if settings.meeting_capture_enabled {
            manager.start();
        }
        manager
    }

    fn listener(&self) -> MutexGuard<'_, Listener> {
        self.listener
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// Turns the endpoint on or off and remembers the choice. The first time
    /// it is turned on, the pairing token is made. Turning it off finishes a
    /// meeting in progress.
    pub fn set_enabled(&self, enabled: bool) -> Result<MeetingCaptureStatus> {
        let app = &self.app;
        let mut settings = get_settings(app);
        if enabled && settings.meeting_capture_token.is_empty() {
            settings.meeting_capture_token = new_token()?;
        }
        settings.meeting_capture_enabled = enabled;
        *self.shared.token() = settings.meeting_capture_token.clone();
        write_settings(app, settings);
        if enabled {
            self.start();
        } else {
            self.stop();
        }
        Ok(self.status())
    }

    /// Replaces the pairing token; an extension with the old one is turned
    /// away until it is given the new one.
    pub fn regenerate_token(&self) -> Result<MeetingCaptureStatus> {
        let app = &self.app;
        let mut settings = get_settings(app);
        settings.meeting_capture_token = new_token()?;
        *self.shared.token() = settings.meeting_capture_token.clone();
        write_settings(app, settings);
        Ok(self.status())
    }

    pub fn status(&self) -> MeetingCaptureStatus {
        let enabled = get_settings(&self.app).meeting_capture_enabled;
        let (listening, error) = {
            let listener = self.listener();
            (listener.running.is_some(), listener.error.clone())
        };
        let recording_seconds = self
            .shared
            .session()
            .as_ref()
            .map(|session| session.seconds() as u32);
        MeetingCaptureStatus {
            enabled,
            listening,
            port: PORT,
            token: self.shared.token().clone(),
            error,
            recording_seconds,
        }
    }

    fn start(&self) {
        let mut listener = self.listener();
        if listener.running.is_some() {
            return;
        }
        let started = Server::http(("127.0.0.1", PORT))
            .map_err(|e| anyhow!("{e}"))
            .and_then(|server| {
                let server = Arc::new(server);
                let stop = Arc::new(AtomicBool::new(false));
                let thread = thread::Builder::new()
                    .name("meeting-capture".into())
                    .spawn({
                        let server = Arc::clone(&server);
                        let shared = Arc::clone(&self.shared);
                        let stop = Arc::clone(&stop);
                        move || server::run(&server, &shared, &stop)
                    })?;
                Ok(Running {
                    server,
                    stop,
                    thread,
                })
            });
        match started {
            Ok(running) => {
                info!("Meeting capture is listening on 127.0.0.1:{PORT}");
                listener.running = Some(running);
                listener.error = None;
            }
            Err(e) => {
                warn!("Meeting capture could not listen on 127.0.0.1:{PORT}: {e:#}");
                listener.error = Some(format!("{e:#}"));
            }
        }
    }

    fn stop(&self) {
        let running = {
            let mut listener = self.listener();
            listener.error = None;
            listener.running.take()
        };
        let Some(Running {
            server,
            stop,
            thread,
        }) = running
        else {
            return;
        };
        stop.store(true, Ordering::Release);
        server.unblock();
        // The thread finishes a meeting in progress on its way out.
        if thread.join().is_err() {
            warn!("The meeting capture thread panicked");
        }
        info!("Meeting capture stopped listening");
    }
}

/// The secret an extension has to present: 32 random bytes.
fn new_token() -> Result<String> {
    random_hex(32)
}

/// `bytes` random bytes from the operating system, as lowercase hex.
fn random_hex(bytes: usize) -> Result<String> {
    let mut random = vec![0u8; bytes];
    getrandom::fill(&mut random).map_err(|e| anyhow!("No random bytes available: {e}"))?;
    Ok(random.iter().map(|byte| format!("{byte:02x}")).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokens_are_long_hex_and_differ() {
        let (a, b) = (new_token().unwrap(), new_token().unwrap());
        assert_eq!(a.len(), 64);
        assert!(a.bytes().all(|b| b.is_ascii_hexdigit()));
        assert_ne!(a, b);
    }
}
