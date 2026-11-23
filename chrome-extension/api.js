// The Whispr app's meeting capture endpoint, which only listens on this
// computer. Every request carries the pairing token from Whispr's settings.

export const DEFAULT_PORT = 47821;

/** A failure with a message fit to show, and whether trying again can help. */
export class WhisprError extends Error {
  constructor(code, message, { retryable = false, nextSeq } = {}) {
    super(message);
    this.code = code;
    this.retryable = retryable;
    this.nextSeq = nextSeq;
  }
}

const MESSAGES = {
  unreachable:
    "Whispr isn't reachable. Open Whispr and turn on Meeting Capture under Settings > Advanced.",
  unauthorized:
    "The pairing token doesn't match. Copy it again from Whispr's settings.",
  meeting_in_progress: "Whispr is already recording a meeting.",
  meeting_too_long: "The meeting reached the longest Whispr records.",
  unknown_meeting: "Whispr no longer has this meeting.",
  not_saved: "Whispr got no audio to save.",
};

export class WhisprClient {
  constructor(port, token) {
    this.base = `http://127.0.0.1:${port || DEFAULT_PORT}/v1`;
    this.token = token;
  }

  async request(method, path, body, contentType) {
    let response;
    try {
      response = await fetch(this.base + path, {
        method,
        headers: {
          Authorization: `Bearer ${this.token}`,
          ...(contentType ? { "Content-Type": contentType } : {}),
        },
        body,
      });
    } catch {
      throw new WhisprError("unreachable", MESSAGES.unreachable, {
        retryable: true,
      });
    }
    const data = await response.json().catch(() => ({}));
    if (response.ok) return data;
    const code = data.error || `http_${response.status}`;
    throw new WhisprError(
      code,
      MESSAGES[code] ||
        `Whispr answered with an error (${code}${data.detail ? `: ${data.detail}` : ""}).`,
      { retryable: response.status >= 500, nextSeq: data.next_seq },
    );
  }

  /** Confirms Whispr is listening and accepts the token. */
  status() {
    return this.request("GET", "/status");
  }

  startMeeting(title) {
    return this.request(
      "POST",
      "/meetings",
      JSON.stringify({ title }),
      "application/json",
    );
  }

  /** Adds audio; `seq` counts from 0 and each one is sent in order. */
  sendAudio(id, seq, pcm) {
    return this.request(
      "POST",
      `/meetings/${id}/audio?seq=${seq}`,
      pcm,
      "application/octet-stream",
    );
  }

  finishMeeting(id) {
    return this.request("POST", `/meetings/${id}/finish`);
  }

  discardMeeting(id) {
    return this.request("DELETE", `/meetings/${id}`);
  }
}
