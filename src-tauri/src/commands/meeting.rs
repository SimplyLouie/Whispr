use std::sync::Arc;

use tauri::State;

use crate::meeting::{MeetingCaptureManager, MeetingCaptureStatus};

/// Whether meeting capture is on and listening, its pairing token, and how
/// much of a meeting in progress is recorded.
#[tauri::command]
#[specta::specta]
pub fn get_meeting_capture_status(
    manager: State<'_, Arc<MeetingCaptureManager>>,
) -> MeetingCaptureStatus {
    manager.status()
}

/// Turns the endpoint the browser extension sends meetings to on or off.
/// Turning it off finishes a meeting in progress.
#[tauri::command]
#[specta::specta]
pub async fn set_meeting_capture_enabled(
    manager: State<'_, Arc<MeetingCaptureManager>>,
    enabled: bool,
) -> Result<MeetingCaptureStatus, String> {
    let manager = Arc::clone(&manager);
    // Stopping waits for the endpoint's thread, which may be saving a meeting.
    tauri::async_runtime::spawn_blocking(move || manager.set_enabled(enabled))
        .await
        .map_err(|e| e.to_string())?
        .map_err(|e| format!("{e:#}"))
}

/// Replaces the pairing token; the extension needs the new one afterwards.
#[tauri::command]
#[specta::specta]
pub fn regenerate_meeting_capture_token(
    manager: State<'_, Arc<MeetingCaptureManager>>,
) -> Result<MeetingCaptureStatus, String> {
    manager.regenerate_token().map_err(|e| format!("{e:#}"))
}
