//! A meeting recording's transcript: who said what, in order.
//!
//! A meeting recording has two channels: the user's microphone on the left
//! and the meeting tab's audio on the right. Each channel holds one side of
//! the conversation, so the pauses in it are where the other side spoke. Each
//! channel is cut into turns at those pauses, every turn is transcribed on its
//! own, and the turns of both channels are put back in the order they started.

use std::ops::Range;
use std::path::Path;

use anyhow::{Context, Result};
use hound::WavReader;
use log::{info, warn};
use tauri::AppHandle;

use crate::audio_toolkit::chunking::{join_transcripts, speech_segments};
use crate::audio_toolkit::SpeechScorer;
use crate::language_detection::LanguageDecision;
use crate::managers::audio::create_detector;
use crate::managers::transcription::{ChunkControl, Transcription, TranscriptionManager};
use crate::settings::get_settings;

const RATE: usize = 16_000;

/// A pause this long in a channel ends a turn.
const TURN_GAP: usize = RATE * 8 / 10;
/// Speech separated by shorter pauses stops being merged into one turn past
/// this length, which keeps each turn inside what a model decodes well.
const MAX_TURN: usize = 30 * RATE;
/// Audio kept on each side of a turn, so its first and last sounds survive.
const TURN_PAD: usize = RATE / 5;
/// A channel that never gets louder than this is not transcribed: a muted or
/// missing microphone, or a tab that played nothing.
const SILENCE_PEAK: i16 = 64;
/// Consecutive turns by one speaker share a paragraph until it spans this much.
const PARAGRAPH_SPAN: usize = 60 * RATE;
/// Frames scored per call to the voice activity detector.
const SCORE_FRAMES: usize = 512;

/// Which side of the meeting a channel holds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Speaker {
    /// The microphone, on the left channel.
    Me,
    /// The meeting tab, on the right channel.
    Others,
}

impl Speaker {
    const ALL: [Speaker; 2] = [Speaker::Me, Speaker::Others];

    fn label(self) -> &'static str {
        match self {
            Speaker::Me => "Me",
            Speaker::Others => "Others",
        }
    }
}

/// One turn's text and where it sits in the meeting, in samples.
struct Spoken {
    start: usize,
    end: usize,
    speaker: Speaker,
    text: String,
}

/// Whether `recording` is a meeting: the only two-channel recordings Handy
/// keeps. Dictations and transcribed files are saved with one channel.
pub fn is_meeting_recording(recording: &Path) -> bool {
    WavReader::open(recording).is_ok_and(|reader| reader.spec().channels == 2)
}

/// Transcribes a meeting recording with the selected model into paragraphs
/// labelled by speaker. `None` if `control` stopped it; empty text if nobody
/// spoke.
pub fn transcribe(
    app: &AppHandle,
    tm: &TranscriptionManager,
    recording: &Path,
    control: &dyn ChunkControl,
) -> Result<Option<Transcription>> {
    let mut spoken: Vec<Spoken> = Vec::new();
    // The language of the longest turn stands for the meeting.
    let mut language: Option<(usize, LanguageDecision)> = None;

    for (channel, speaker) in Speaker::ALL.into_iter().enumerate() {
        if !control.proceed() {
            return Ok(None);
        }
        // One channel in memory at a time, as 16-bit samples: an hour is
        // about 115 MB this way.
        let samples = read_channel(recording, channel)
            .with_context(|| format!("Failed to load the meeting's {} audio", speaker.label()))?;
        let turns = if is_silent(&samples) {
            Vec::new()
        } else {
            find_turns(app, &samples)
        };
        info!(
            "Meeting channel '{}': {:.0}s of audio in {} turns",
            speaker.label(),
            samples.len() as f64 / RATE as f64,
            turns.len()
        );

        let total: usize = turns.iter().map(Range::len).sum();
        let mut done = 0;
        for turn in turns {
            let progress = ChannelProgress {
                control,
                channel,
                done,
                total,
            };
            let Some(transcription) =
                tm.transcribe_in_chunks(to_f32(&samples[turn.clone()]), &progress)?
            else {
                return Ok(None);
            };
            done += turn.len();
            if transcription.text.trim().is_empty() {
                continue;
            }
            if let Some(decision) = transcription.language {
                if language
                    .as_ref()
                    .is_none_or(|(longest, _)| turn.len() > *longest)
                {
                    language = Some((turn.len(), decision));
                }
            }
            spoken.push(Spoken {
                start: turn.start,
                end: turn.end,
                speaker,
                text: transcription.text,
            });
        }
        control.progress(channel + 1, Speaker::ALL.len());
    }

    Ok(Some(Transcription {
        text: format_transcript(spoken),
        language: language.map(|(_, decision)| decision),
    }))
}

/// Reports one channel's turns as their share of the whole meeting.
struct ChannelProgress<'a> {
    control: &'a dyn ChunkControl,
    channel: usize,
    /// Samples of this channel's turns already transcribed.
    done: usize,
    /// Samples in all of this channel's turns.
    total: usize,
}

impl ChunkControl for ChannelProgress<'_> {
    fn proceed(&self) -> bool {
        self.control.proceed()
    }

    fn progress(&self, done: usize, _total: usize) {
        const SCALE: usize = 1000;
        let within = (self.done + done).min(self.total) * SCALE / self.total.max(1);
        self.control
            .progress(self.channel * SCALE + within, Speaker::ALL.len() * SCALE);
    }
}

/// One channel of a recording.
fn read_channel(recording: &Path, channel: usize) -> Result<Vec<i16>> {
    let reader = WavReader::open(recording)?;
    let channels = reader.spec().channels as usize;
    let samples = reader
        .into_samples::<i16>()
        .skip(channel)
        .step_by(channels.max(1))
        .collect::<Result<Vec<i16>, _>>()?;
    Ok(samples)
}

fn to_f32(samples: &[i16]) -> Vec<f32> {
    samples
        .iter()
        .map(|&sample| sample as f32 / i16::MAX as f32)
        .collect()
}

fn is_silent(samples: &[i16]) -> bool {
    samples
        .iter()
        .all(|sample| sample.unsigned_abs() <= SILENCE_PEAK.unsigned_abs())
}

/// The channel's turns, in order. If the voice activity detector can't run,
/// the channel is cut into equal pieces instead, so it is still transcribed.
fn find_turns(app: &AppHandle, samples: &[i16]) -> Vec<Range<usize>> {
    match speech_probabilities(app, samples) {
        Ok((probs, frame_samples)) => turns(
            &speech_segments(&probs, frame_samples, samples.len()),
            samples.len(),
        ),
        Err(e) => {
            warn!("Voice activity detection failed on a meeting channel; cutting it evenly: {e:#}");
            (0..samples.len())
                .step_by(MAX_TURN)
                .map(|start| start..(start + MAX_TURN).min(samples.len()))
                .collect()
        }
    }
}

/// One speech probability per detector frame, and the frame size.
fn speech_probabilities(app: &AppHandle, samples: &[i16]) -> Result<(Vec<f32>, usize)> {
    let mut scorer = create_detector(get_settings(app).vad_backend, app)?;
    let probs = score(scorer.as_mut(), samples)?;
    Ok((probs, scorer.frame_samples()))
}

/// One speech probability per frame of `samples`, a partial last frame scored
/// zero padded.
fn score(scorer: &mut dyn SpeechScorer, samples: &[i16]) -> Result<Vec<f32>> {
    let frame_samples = scorer.frame_samples();
    anyhow::ensure!(frame_samples > 0, "VAD frame size must be non-zero");
    let mut probs = Vec::with_capacity(samples.len() / frame_samples + 1);
    for piece in samples.chunks(frame_samples * SCORE_FRAMES) {
        let mut pcm = to_f32(piece);
        // Only the last piece can end in a partial frame.
        let partial = pcm.len() % frame_samples;
        if partial != 0 {
            pcm.resize(pcm.len() + frame_samples - partial, 0.0);
        }
        probs.extend(scorer.score(&pcm)?);
    }
    Ok(probs)
}

/// Speech segments joined into turns: segments less than [`TURN_GAP`] apart
/// are one turn, up to [`MAX_TURN`]. Each turn is then widened by up to
/// [`TURN_PAD`] without reaching into its neighbours. `len` is the channel's
/// length.
fn turns(segments: &[Range<usize>], len: usize) -> Vec<Range<usize>> {
    let mut turns: Vec<Range<usize>> = Vec::new();
    for segment in segments {
        if let Some(last) = turns.last_mut() {
            if segment.start.saturating_sub(last.end) < TURN_GAP
                && segment.end.saturating_sub(last.start) <= MAX_TURN
            {
                last.end = last.end.max(segment.end);
                continue;
            }
        }
        turns.push(segment.clone());
    }

    let bare = turns.clone();
    for (i, turn) in turns.iter_mut().enumerate() {
        // Room to grow into: up to the audio's edge, or half the pause shared
        // with a neighbour.
        let before = match i.checked_sub(1) {
            Some(previous) => turn.start.saturating_sub(bare[previous].end) / 2,
            None => turn.start,
        };
        let after = match bare.get(i + 1) {
            Some(next) => next.start.saturating_sub(turn.end) / 2,
            None => len.saturating_sub(turn.end),
        };
        turn.start -= before.min(TURN_PAD);
        turn.end += after.min(TURN_PAD);
    }
    turns
}

/// Every turn in the order it started, one paragraph per stretch by the same
/// speaker: `[12:34] Me: ...`.
fn format_transcript(mut spoken: Vec<Spoken>) -> String {
    spoken.sort_by_key(|turn| turn.start);

    struct Paragraph {
        start: usize,
        speaker: Speaker,
        texts: Vec<String>,
    }
    let mut paragraphs: Vec<Paragraph> = Vec::new();
    for turn in spoken {
        // A turn long enough to be decoded in chunks comes back as several
        // paragraphs; here it is one speaker's stretch.
        let texts = turn
            .text
            .split("\n\n")
            .map(str::trim)
            .filter(|text| !text.is_empty())
            .map(str::to_owned);
        match paragraphs.last_mut() {
            Some(last)
                if last.speaker == turn.speaker
                    && turn.end.saturating_sub(last.start) <= PARAGRAPH_SPAN =>
            {
                last.texts.extend(texts);
            }
            _ => paragraphs.push(Paragraph {
                start: turn.start,
                speaker: turn.speaker,
                texts: texts.collect(),
            }),
        }
    }

    paragraphs
        .iter()
        .map(|paragraph| {
            format!(
                "[{}] {}: {}",
                timestamp(paragraph.start),
                paragraph.speaker.label(),
                join_transcripts(&paragraph.texts)
            )
        })
        .collect::<Vec<_>>()
        .join("\n\n")
}

/// A position in the meeting as `mm:ss`, or `h:mm:ss` from an hour on.
fn timestamp(sample: usize) -> String {
    let seconds = sample / RATE;
    let (hours, minutes, seconds) = (seconds / 3600, seconds / 60 % 60, seconds % 60);
    if hours > 0 {
        format!("{hours}:{minutes:02}:{seconds:02}")
    } else {
        format!("{minutes:02}:{seconds:02}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(seconds: f64) -> usize {
        (seconds * RATE as f64).round() as usize
    }

    fn said(start: f64, end: f64, speaker: Speaker, text: &str) -> Spoken {
        Spoken {
            start: s(start),
            end: s(end),
            speaker,
            text: text.to_string(),
        }
    }

    #[test]
    fn short_pauses_stay_in_one_turn() {
        // 0.5 s apart: one turn. 2 s apart: a new one.
        let segments = [s(1.0)..s(3.0), s(3.5)..s(5.0), s(7.0)..s(9.0)];
        let turns = turns(&segments, s(20.0));
        assert_eq!(turns.len(), 2);
        assert_eq!(turns[0], s(1.0) - TURN_PAD..s(5.0) + TURN_PAD);
        assert_eq!(turns[1], s(7.0) - TURN_PAD..s(9.0) + TURN_PAD);
    }

    #[test]
    fn a_turn_stops_growing_at_its_maximum() {
        // Speech with 0.5 s pauses for 50 s: cut once the turn would pass 30 s.
        let segments: Vec<Range<usize>> = (0..10)
            .map(|i| s(i as f64 * 5.0)..s(i as f64 * 5.0 + 4.5))
            .collect();
        let turns = turns(&segments, s(50.0));
        assert_eq!(turns.len(), 2);
        assert!(turns
            .iter()
            .all(|turn| turn.len() <= MAX_TURN + 2 * TURN_PAD));
        assert_eq!(turns[0].start, 0, "padding never goes before the start");
    }

    #[test]
    fn padding_stays_out_of_neighbours_and_the_end() {
        // 0.9 s apart: separate turns, with less room than two pads between.
        let segments = [s(1.0)..s(2.0), s(2.9)..s(4.0)];
        let turns = turns(&segments, s(4.05));
        assert_eq!(turns.len(), 2);
        assert!(turns[0].end <= turns[1].start, "turns never overlap");
        assert_eq!(turns[1].end, s(4.05), "padding never passes the end");
    }

    #[test]
    fn turns_are_ordered_by_start_and_grouped_by_speaker() {
        let transcript = format_transcript(vec![
            said(12.0, 15.0, Speaker::Others, "Sounds good."),
            said(0.0, 4.0, Speaker::Me, "Hi everyone."),
            said(5.0, 8.0, Speaker::Me, "Shall we start?"),
            said(3725.0, 3730.0, Speaker::Me, "Thanks, bye."),
        ]);
        assert_eq!(
            transcript,
            "[00:00] Me: Hi everyone. Shall we start?\n\n\
             [00:12] Others: Sounds good.\n\n\
             [1:02:05] Me: Thanks, bye."
        );
    }

    #[test]
    fn a_long_stretch_by_one_speaker_gets_new_paragraphs() {
        let transcript = format_transcript(vec![
            said(0.0, 30.0, Speaker::Others, "First part."),
            said(31.0, 59.0, Speaker::Others, "Second part."),
            said(61.0, 90.0, Speaker::Others, "Third part."),
        ]);
        assert_eq!(
            transcript,
            "[00:00] Others: First part. Second part.\n\n[01:01] Others: Third part."
        );
    }

    #[test]
    fn a_chunked_turn_reads_as_one_stretch() {
        let transcript = format_transcript(vec![said(0.0, 50.0, Speaker::Me, "One.\n\nTwo.\n\n")]);
        assert_eq!(transcript, "[00:00] Me: One. Two.");
    }

    #[test]
    fn near_silence_is_not_transcribed() {
        assert!(is_silent(&[0, 12, -40, 64, -64]));
        assert!(!is_silent(&[0, 12, -900]));
        assert!(is_silent(&[]));
    }
}
