//! One meeting being recorded to disk, and how it becomes a History entry.

use std::fs::{self, File};
use std::io::BufWriter;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{bail, Context, Result};
use hound::{SampleFormat, WavReader, WavSpec, WavWriter};
use log::{error, info, warn};
use tauri::{AppHandle, Manager};

use crate::managers::history::HistoryManager;
use crate::managers::transcription_jobs::TranscriptionJobManager;

const RATE: u32 = 16_000;
/// The microphone on the left, the meeting tab on the right.
const CHANNELS: u16 = 2;
/// Bytes of one frame of the audio the extension sends: 16-bit, two channels.
pub(super) const FRAME_BYTES: usize = 2 * CHANNELS as usize;
/// Shorter recordings are dropped: a start that was stopped straight away.
const MIN_FRAMES: u64 = RATE as u64;
/// A meeting is closed once it is this long.
pub(super) const MAX_FRAMES: u64 = 6 * 60 * 60 * RATE as u64;

const FILE_PREFIX: &str = "handy-meeting-";
const PARTIAL_SUFFIX: &str = ".partial";
const DEFAULT_TITLE: &str = "Meeting";
const RECOVERED_TITLE: &str = "Meeting (recovered)";
const MAX_TITLE_CHARS: usize = 120;

const WAV_SPEC: WavSpec = WavSpec {
    channels: CHANNELS,
    sample_rate: RATE,
    bits_per_sample: 16,
    sample_format: SampleFormat::Int,
};

/// Where recordings are kept and what takes a finished one: History in the
/// app, a plain folder in tests.
pub(super) trait MeetingStore: Send + Sync {
    fn recordings_dir(&self) -> PathBuf;

    /// Takes the recording named `file_name` in the recordings folder and
    /// queues its transcription. Returns what identifies it from then on.
    fn add(&self, file_name: String, title: String) -> Result<i64>;
}

/// The app's store: a meeting becomes a History entry and a transcription job.
pub(super) struct HistoryStore(pub(super) AppHandle);

impl MeetingStore for HistoryStore {
    fn recordings_dir(&self) -> PathBuf {
        self.0
            .state::<Arc<HistoryManager>>()
            .recordings_dir()
            .to_path_buf()
    }

    fn add(&self, file_name: String, title: String) -> Result<i64> {
        let entry = self
            .0
            .state::<Arc<HistoryManager>>()
            .save_file_entry(file_name, title)?;
        self.0
            .state::<Arc<TranscriptionJobManager>>()
            .queue_recording(entry.id);
        Ok(entry.id)
    }
}

/// A meeting whose audio is still arriving. It is written to a `.partial`
/// file that only takes its real name once the meeting is in History, and the
/// file is a complete WAV after every append, so [`recover_unfinished`] can
/// pick it up if Handy stops first.
pub(super) struct Session {
    pub(super) id: String,
    title: String,
    file_name: String,
    partial: PathBuf,
    writer: WavWriter<BufWriter<File>>,
    frames: u64,
    /// The sequence number the next audio must carry.
    pub(super) next_seq: u64,
    last_activity: Instant,
}

impl Session {
    pub(super) fn start(store: &dyn MeetingStore, id: String, title: &str) -> Result<Self> {
        // The id keeps two meetings started in the same millisecond apart.
        let file_name = format!(
            "{FILE_PREFIX}{}-{}.wav",
            chrono::Utc::now().timestamp_millis(),
            id.chars().take(8).collect::<String>()
        );
        let partial = partial_path(&store.recordings_dir().join(&file_name));
        let writer = WavWriter::create(&partial, WAV_SPEC)
            .with_context(|| format!("Failed to create {}", partial.display()))?;
        Ok(Self {
            id,
            title: clean_title(title),
            file_name,
            partial,
            writer,
            frames: 0,
            next_seq: 0,
            last_activity: Instant::now(),
        })
    }

    /// Adds whole frames of little-endian 16-bit audio.
    pub(super) fn append(&mut self, pcm: &[u8]) -> Result<()> {
        debug_assert!(pcm.len().is_multiple_of(FRAME_BYTES));
        let (samples, _) = pcm.as_chunks::<2>();
        for sample in samples {
            self.writer.write_sample(i16::from_le_bytes(*sample))?;
        }
        // Also rewrites the header, so the file is valid up to here.
        self.writer.flush()?;
        self.frames += (pcm.len() / FRAME_BYTES) as u64;
        self.next_seq += 1;
        self.last_activity = Instant::now();
        Ok(())
    }

    pub(super) fn frames(&self) -> u64 {
        self.frames
    }

    pub(super) fn seconds(&self) -> f64 {
        self.frames as f64 / RATE as f64
    }

    /// How long since the meeting started or last got audio.
    pub(super) fn idle_for(&self) -> Duration {
        self.last_activity.elapsed()
    }

    /// Closes the recording, adds it to History and queues its transcription.
    pub(super) fn finish(self, store: &dyn MeetingStore) -> Result<i64> {
        let Session {
            title,
            file_name,
            partial,
            writer,
            frames,
            ..
        } = self;
        // A recording that can't be closed stays as it is, to be recovered.
        writer
            .finalize()
            .context("Failed to finish the meeting recording")?;
        add_to_history(store, &partial, file_name, title, frames)
    }

    /// Drops the meeting and its recording.
    pub(super) fn discard(self) {
        let Session {
            partial, writer, ..
        } = self;
        drop(writer);
        remove(&partial);
    }
}

/// Meetings left behind when Handy stopped mid-recording go to History with
/// the audio they had.
pub(super) fn recover_unfinished(store: &dyn MeetingStore) {
    let Ok(dir) = fs::read_dir(store.recordings_dir()) else {
        return;
    };
    for file in dir.flatten() {
        let name = file.file_name().to_string_lossy().into_owned();
        let Some(file_name) = name
            .strip_suffix(PARTIAL_SUFFIX)
            .filter(|name| name.starts_with(FILE_PREFIX))
        else {
            continue;
        };
        let path = file.path();
        let frames = WavReader::open(&path).map_or(0, |reader| reader.duration() as u64);
        match add_to_history(
            store,
            &path,
            file_name.to_string(),
            RECOVERED_TITLE.to_string(),
            frames,
        ) {
            Ok(id) => info!("Recovered an unfinished meeting as history entry {id}"),
            Err(e) => warn!("Did not recover {}: {e:#}", path.display()),
        }
    }
}

/// Gives the closed recording at `partial` its real name and hands it to the
/// store. A recording too short to keep is deleted.
fn add_to_history(
    store: &dyn MeetingStore,
    partial: &Path,
    file_name: String,
    title: String,
    frames: u64,
) -> Result<i64> {
    if frames < MIN_FRAMES {
        remove(partial);
        bail!("The meeting has no audio");
    }
    let recording = store.recordings_dir().join(&file_name);
    fs::rename(partial, &recording).context("Failed to save the meeting recording")?;
    match store.add(file_name, title) {
        Ok(id) => {
            info!(
                "Queued a {:.0}s meeting for transcription as history entry {id}",
                frames as f64 / RATE as f64
            );
            Ok(id)
        }
        Err(e) => {
            // Back to its partial name: the next start tries again.
            if let Err(e) = fs::rename(&recording, partial) {
                error!("Failed to keep {} for recovery: {e}", recording.display());
            }
            Err(e)
        }
    }
}

fn partial_path(recording: &Path) -> PathBuf {
    let mut name = recording.as_os_str().to_owned();
    name.push(PARTIAL_SUFFIX);
    PathBuf::from(name)
}

/// The title as History shows it: one line of bounded length.
fn clean_title(title: &str) -> String {
    let title: String = title
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .take(MAX_TITLE_CHARS)
        .collect();
    if title.is_empty() {
        DEFAULT_TITLE.to_string()
    } else {
        title
    }
}

fn remove(path: &Path) {
    if let Err(e) = fs::remove_file(path) {
        if e.kind() != std::io::ErrorKind::NotFound {
            error!("Failed to remove {}: {e}", path.display());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn titles_are_one_bounded_line() {
        assert_eq!(
            clean_title("  Weekly\n sync \t- Meet "),
            "Weekly sync - Meet"
        );
        assert_eq!(clean_title(" \n "), DEFAULT_TITLE);
        assert_eq!(
            clean_title(&"x".repeat(500)).chars().count(),
            MAX_TITLE_CHARS
        );
    }

    #[test]
    fn a_partial_recording_keeps_its_name_as_a_prefix() {
        assert_eq!(
            partial_path(Path::new("/r/handy-meeting-1.wav")),
            Path::new("/r/handy-meeting-1.wav.partial")
        );
    }
}
