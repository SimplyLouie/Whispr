// Runs on the audio thread of a 16 kHz context. Input 0 is the microphone,
// input 1 is the meeting tab; both are mixed down to one channel before they
// get here. Posts them interleaved as 16-bit samples, microphone first, half a
// second at a time: the format Whispr stores.

const FRAMES_PER_MESSAGE = 8000;
/** Frames per call to `process` when no input says otherwise. */
const RENDER_QUANTUM = 128;

const toInt16 = (sample) => {
  const clamped = Math.max(-1, Math.min(1, sample));
  return clamped < 0 ? clamped * 0x8000 : clamped * 0x7fff;
};

class MeetingRecorder extends AudioWorkletProcessor {
  constructor() {
    super();
    this.buffer = new Int16Array(FRAMES_PER_MESSAGE * 2);
    this.filled = 0;
    this.port.onmessage = (event) => {
      if (event.data === "flush") {
        this.post();
        this.port.postMessage({ flushed: true });
      }
    };
  }

  post() {
    if (this.filled === 0) return;
    const pcm = this.buffer.slice(0, this.filled * 2);
    this.port.postMessage({ pcm: pcm.buffer }, [pcm.buffer]);
    this.filled = 0;
  }

  process(inputs) {
    // An input with nothing connected, or whose source ended, has no channels.
    // Silence is still written for it, so the recording keeps real time.
    const microphone = inputs[0][0];
    const tab = inputs[1][0];
    const frames = (microphone || tab)?.length ?? RENDER_QUANTUM;
    for (let i = 0; i < frames; i++) {
      this.buffer[this.filled * 2] = microphone ? toInt16(microphone[i]) : 0;
      this.buffer[this.filled * 2 + 1] = tab ? toInt16(tab[i]) : 0;
      if (++this.filled === FRAMES_PER_MESSAGE) this.post();
    }
    return true;
  }
}

registerProcessor("meeting-recorder", MeetingRecorder);
