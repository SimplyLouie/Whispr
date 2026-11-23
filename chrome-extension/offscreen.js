// The hidden page that records a meeting: a service worker can't hold media
// streams, so the background worker opens this page and tells it what to do.
// It captures the meeting tab and the microphone, and streams both to Whispr.

import { WhisprClient, WhisprError } from "./api.js";

/** 16 kHz, two channels, 16-bit. */
const BYTES_PER_SECOND = 16000 * 2 * 2;
/** Audio is sent once this much is waiting. */
const SEND_AFTER_BYTES = 2 * BYTES_PER_SECOND;
/** The most sent at once, when audio piled up while Whispr didn't answer. */
const MAX_SEND_BYTES = 30 * BYTES_PER_SECOND;
/** How much may pile up before the recording gives up on Whispr. */
const MAX_WAITING_BYTES = 20 * 60 * BYTES_PER_SECOND;
const RETRY_MS = 2000;
/** How long stopping waits for audio Whispr hasn't taken yet. */
const DRAIN_MS = 20000;
const FLUSH_MS = 500;
/** How long the audio contexts get to start running. */
const START_MS = 2000;

const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms));

/** The meeting being recorded, if any. */
let recording = null;

class Recording {
  constructor(client) {
    this.client = client;
    this.id = null;
    this.micActive = false;
    /** Audio not sent yet, as the worklet posted it. */
    this.waiting = [];
    this.waitingBytes = 0;
    this.capturedBytes = 0;
    this.sentBytes = 0;
    this.seq = 0;
    /** Whether the tab and microphone are still being captured. */
    this.capturing = false;
    this.ending = false;
    /** Why sending stopped for good. */
    this.failure = null;
    /** Why sending is being retried. */
    this.warning = null;
    this.deadline = null;
    this.sending = null;
    this.streams = [];
    this.contexts = [];
  }

  /** Starts capturing; audio waits until the meeting has an id. */
  async capture(tabStream, micStream) {
    this.streams = [tabStream, micStream].filter(Boolean);
    this.micActive = Boolean(micStream);

    // A captured tab goes silent; play it back so the meeting stays audible.
    const playback = new AudioContext();
    playback.createMediaStreamSource(tabStream).connect(playback.destination);

    // A second context at 16 kHz resamples both sources on one clock, so the
    // two channels stay in step.
    const context = new AudioContext({ sampleRate: 16000 });
    this.contexts = [playback, context];
    await context.audioWorklet.addModule("recorder-worklet.js");
    this.node = new AudioWorkletNode(context, "meeting-recorder", {
      numberOfInputs: 2,
      numberOfOutputs: 1,
      outputChannelCount: [1],
      channelCount: 1,
      channelCountMode: "explicit",
      channelInterpretation: "speakers",
    });
    if (micStream) {
      context.createMediaStreamSource(micStream).connect(this.node, 0, 0);
    }
    context.createMediaStreamSource(tabStream).connect(this.node, 0, 1);
    // A node only runs while it leads to the output; this path is silent.
    const silent = context.createGain();
    silent.gain.value = 0;
    this.node.connect(silent).connect(context.destination);

    this.node.port.onmessage = ({ data }) => {
      if (data.pcm) this.add(data.pcm);
      if (data.flushed) this.flushed?.();
    };
    this.capturing = true;
    // Resuming never settles if Chrome won't let this page make sound.
    await Promise.race([
      Promise.all([playback.resume(), context.resume()]),
      sleep(START_MS),
    ]);
    if (context.state !== "running") {
      throw new Error("Chrome didn't let the recorder start its audio.");
    }

    tabStream
      .getAudioTracks()[0]
      ?.addEventListener("ended", () => endOnItsOwn("The meeting tab closed."));
  }

  add(pcm) {
    if (!this.capturing || this.failure) return;
    this.waiting.push(pcm);
    this.waitingBytes += pcm.byteLength;
    this.capturedBytes += pcm.byteLength;
    if (this.waitingBytes > MAX_WAITING_BYTES) {
      this.failure = new WhisprError(
        "gave_up",
        "Whispr stopped answering, so the recording ended.",
      );
    }
  }

  /** Starts sending to the meeting `id`. */
  begin(id) {
    this.id = id;
    this.sending = this.send().then(() => {
      if (!this.failure || this.ending) return;
      // Whispr saved a meeting it closed for length; say why it ended.
      const tooLong = this.failure.code === "meeting_too_long";
      endOnItsOwn(tooLong ? this.failure.message : "");
    });
  }

  /** Up to `MAX_SEND_BYTES` of the waiting audio, in one piece. */
  take() {
    const parts = [];
    let bytes = 0;
    while (
      this.waiting.length > 0 &&
      (parts.length === 0 ||
        bytes + this.waiting[0].byteLength <= MAX_SEND_BYTES)
    ) {
      const part = this.waiting.shift();
      parts.push(part);
      bytes += part.byteLength;
    }
    this.waitingBytes -= bytes;
    const body = new Uint8Array(bytes);
    let at = 0;
    for (const part of parts) {
      body.set(new Uint8Array(part), at);
      at += part.byteLength;
    }
    return body;
  }

  /** Sends the waiting audio in order until capture ends and none is left. */
  async send() {
    while (!this.failure) {
      const due = this.capturing
        ? this.waitingBytes >= SEND_AFTER_BYTES
        : this.waitingBytes > 0;
      if (!due) {
        if (!this.capturing) return;
        await sleep(250);
        continue;
      }
      const body = this.take();
      // The same piece under the same number until Whispr has it.
      for (;;) {
        try {
          await this.client.sendAudio(this.id, this.seq, body);
        } catch (error) {
          // An answer that got lost: Whispr already wrote this piece.
          const landed =
            error.code === "out_of_order" && error.nextSeq === this.seq + 1;
          if (!landed) {
            const outOfTime = this.deadline && Date.now() > this.deadline;
            if (!error.retryable || outOfTime || this.failure) {
              this.failure ??= error;
              return;
            }
            this.warning = error.message;
            await sleep(RETRY_MS);
            continue;
          }
        }
        this.seq += 1;
        this.sentBytes += body.byteLength;
        this.warning = null;
        break;
      }
    }
  }

  /** Asks the worklet for what it holds, so the last half second isn't lost. */
  async flush() {
    if (!this.node) return;
    const flushed = new Promise((resolve) => (this.flushed = resolve));
    this.node.port.postMessage("flush");
    await Promise.race([flushed, sleep(FLUSH_MS)]);
  }

  release() {
    this.capturing = false;
    for (const stream of this.streams) {
      for (const track of stream.getTracks()) track.stop();
    }
    for (const context of this.contexts) context.close().catch(() => {});
    this.streams = [];
    this.contexts = [];
  }

  /**
   * Ends the recording. Unless it is discarded, the rest of the audio is
   * sent and Whispr queues the meeting for transcription.
   */
  async stop(discard) {
    this.ending = true;
    await this.flush();
    this.release();
    if (discard) {
      this.failure ??= new WhisprError(
        "discarded",
        "The recording was discarded.",
      );
      await this.sending;
      if (this.id) await this.client.discardMeeting(this.id);
      return;
    }
    this.deadline = Date.now() + DRAIN_MS;
    await this.sending;
    // Whispr closed it itself; what it had is already queued.
    if (this.failure?.code === "meeting_too_long") return;
    if (this.failure) throw this.failure;
    await this.client.finishMeeting(this.id);
  }

  state() {
    return {
      micActive: this.micActive,
      seconds: Math.floor(this.capturedBytes / BYTES_PER_SECOND),
      warning: this.warning,
    };
  }
}

async function start({ streamId, port, token, title }) {
  if (recording) throw new Error("A meeting is already being recorded.");
  const client = new WhisprClient(port, token);
  // Before touching the tab: a wrong token or a closed Whispr ends it here.
  await client.status();

  const tabStream = await navigator.mediaDevices.getUserMedia({
    audio: {
      mandatory: {
        chromeMediaSource: "tab",
        chromeMediaSourceId: streamId,
      },
    },
    video: false,
  });
  // Without the microphone only the other participants are recorded. A
  // hidden page can't show Chrome's prompt, so it only asks once the
  // microphone is already allowed.
  const permission = await navigator.permissions
    .query({ name: "microphone" })
    .catch(() => null);
  const micStream =
    permission?.state === "granted"
      ? await navigator.mediaDevices
          .getUserMedia({
            audio: {
              echoCancellation: true,
              noiseSuppression: true,
              autoGainControl: true,
            },
            video: false,
          })
          .catch(() => null)
      : null;

  const starting = new Recording(client);
  try {
    await starting.capture(tabStream, micStream);
    const { id } = await client.startMeeting(title);
    starting.begin(id);
  } catch (error) {
    starting.release();
    throw error;
  }
  recording = starting;
  return { micActive: starting.micActive };
}

async function stop({ discard }) {
  const ended = recording;
  if (!ended) throw new Error("No meeting is being recorded.");
  try {
    await ended.stop(Boolean(discard));
  } finally {
    recording = null;
  }
  return {};
}

/** The recording ended without the user stopping it: tell the background. */
async function endOnItsOwn(reason) {
  const ended = recording;
  if (!ended || ended.ending) return;
  let outcome;
  try {
    await ended.stop(false);
    outcome = { ok: true, message: reason };
  } catch (error) {
    outcome = { ok: false, message: `${reason} ${error.message}`.trim() };
  }
  recording = null;
  chrome.runtime
    .sendMessage({ target: "background", type: "ended", ...outcome })
    .catch(() => {});
}

const handlers = {
  start,
  stop,
  state: async () => (recording ? recording.state() : {}),
};

chrome.runtime.onMessage.addListener((message, _sender, sendResponse) => {
  if (message?.target !== "offscreen") return undefined;
  const handler = handlers[message.type];
  if (!handler) {
    sendResponse({ ok: false, error: "Unknown request." });
    return undefined;
  }
  handler(message).then(
    (data) => sendResponse({ ok: true, ...data }),
    (error) =>
      sendResponse({ ok: false, error: error?.message || String(error) }),
  );
  return true;
});
