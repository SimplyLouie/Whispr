// The extension's service worker. It owns no audio: it gets the tab's stream
// id, opens the hidden recorder page (offscreen.js), and remembers what is
// being recorded so the popup and the badge can show it. It can be shut down
// between messages, so everything it knows is in chrome.storage.session.

import { DEFAULT_PORT } from "./api.js";

const RECORDER_PAGE = "offscreen.html";
const SENT_MESSAGE =
  "Sent to Whispr. The transcript appears in its History once it is transcribed.";

async function hasRecorder() {
  const contexts = await chrome.runtime.getContexts({
    contextTypes: ["OFFSCREEN_DOCUMENT"],
  });
  return contexts.length > 0;
}

async function openRecorder() {
  if (await hasRecorder()) return;
  await chrome.offscreen.createDocument({
    url: RECORDER_PAGE,
    reasons: ["USER_MEDIA"],
    justification:
      "Records the meeting tab and the microphone while a meeting is captured.",
  });
}

async function closeRecorder() {
  if (await hasRecorder()) {
    await chrome.offscreen.closeDocument().catch(() => {});
  }
}

const toRecorder = (message) =>
  chrome.runtime.sendMessage({ ...message, target: "offscreen" });

async function setBadge(recording) {
  await chrome.action.setBadgeBackgroundColor({ color: "#d93025" });
  await chrome.action.setBadgeText({ text: recording ? "REC" : "" });
}

/** Forgets the recording and keeps how it ended for the popup. */
async function settle(last) {
  await chrome.storage.session.set({ recording: null, last });
  await setBadge(false);
  await closeRecorder();
}

async function getState() {
  const { recording = null, last = null } = await chrome.storage.session.get([
    "recording",
    "last",
  ]);
  if (!recording) return { recording: null, last };
  if (!(await hasRecorder())) {
    const stopped = {
      ok: false,
      message:
        "The recording stopped unexpectedly. Whispr keeps the audio it received.",
    };
    await settle(stopped);
    return { recording: null, last: stopped };
  }
  const live = await toRecorder({ type: "state" }).catch(() => null);
  return {
    recording: {
      ...recording,
      seconds: live?.seconds ?? 0,
      warning: live?.warning ?? null,
    },
    last,
  };
}

async function start(tabId) {
  const { recording } = await chrome.storage.session.get("recording");
  if (recording && (await hasRecorder())) {
    throw new Error("A meeting is already being recorded.");
  }
  const { token = "", port = DEFAULT_PORT } = await chrome.storage.local.get([
    "token",
    "port",
  ]);
  if (!token) throw new Error("Paste the pairing token from Whispr first.");

  const tab = await chrome.tabs.get(tabId);
  const title = tab.title || "Meeting";
  await openRecorder();
  // Last, because a stream id has to be used within a few seconds.
  const streamId = await chrome.tabCapture.getMediaStreamId({
    targetTabId: tabId,
  });
  const reply = await toRecorder({
    type: "start",
    streamId,
    token,
    port,
    title,
  });
  if (!reply?.ok) {
    await closeRecorder();
    throw new Error(reply?.error || "The recording could not start.");
  }
  await chrome.storage.session.set({
    recording: { tabId, title, micActive: reply.micActive },
    last: null,
  });
  await setBadge(true);
}

async function stop(discard) {
  const reply = await toRecorder({ type: "stop", discard }).catch((error) => ({
    ok: false,
    error: error.message,
  }));
  if (reply?.ok) {
    await settle({
      ok: true,
      message: discard ? "The recording was discarded." : SENT_MESSAGE,
    });
  } else {
    await settle({
      ok: false,
      message: reply?.error || "The recording could not be stopped cleanly.",
    });
  }
}

const handlers = {
  state: async () => ({ state: await getState() }),
  start: async ({ tabId }) => {
    await start(tabId);
    return { state: await getState() };
  },
  stop: async ({ discard }) => {
    await stop(Boolean(discard));
    return { state: await getState() };
  },
  // The recorder ended on its own: the tab closed, or Whispr turned it away.
  ended: async ({ ok, message }) => {
    await settle({
      ok,
      message: ok ? `${message} ${SENT_MESSAGE}`.trim() : message,
    });
    return {};
  },
};

chrome.runtime.onMessage.addListener((message, _sender, sendResponse) => {
  if (message?.target !== "background") return undefined;
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
