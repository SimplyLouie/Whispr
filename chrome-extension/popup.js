// The popup: pairs the extension with Whispr, and starts and stops a recording
// of the tab it was opened on. Opening it is also what lets Chrome capture
// that tab.

import { DEFAULT_PORT, WhisprClient } from "./api.js";

const $ = (id) => document.getElementById(id);
const show = (id, visible) => {
  $(id).hidden = !visible;
};

const background = async (message) => {
  const reply = await chrome.runtime.sendMessage({
    ...message,
    target: "background",
  });
  if (!reply?.ok)
    throw new Error(reply?.error || "The extension didn't answer.");
  return reply;
};

const formatElapsed = (seconds) => {
  const hours = Math.floor(seconds / 3600);
  const minutes = Math.floor((seconds % 3600) / 60);
  const rest = String(seconds % 60).padStart(2, "0");
  return hours > 0
    ? `${hours}:${String(minutes).padStart(2, "0")}:${rest}`
    : `${minutes}:${rest}`;
};

/** How often the popup asks how the recording is doing. */
const REFRESH_MS = 1000;
let refreshTimer = null;

function showResult(last) {
  show("result", Boolean(last));
  if (!last) return;
  $("result").textContent = last.message;
  $("result").classList.toggle("bad", !last.ok);
}

function render(state) {
  const { recording, last } = state;
  show("idle", !recording);
  show("recording", Boolean(recording));
  showResult(recording ? null : last);
  clearInterval(refreshTimer);
  if (!recording) return;

  $("recording-title").textContent = recording.title;
  $("elapsed").textContent = formatElapsed(recording.seconds);
  show("no-mic", !recording.micActive);
  show("warning", Boolean(recording.warning));
  $("warning").textContent = recording.warning || "";
  refreshTimer = setInterval(refresh, REFRESH_MS);
}

async function refresh() {
  try {
    render((await background({ type: "state" })).state);
  } catch (error) {
    showResult({ ok: false, message: error.message });
  }
}

/** Whether Whispr is listening and takes the token. */
async function checkConnection() {
  const { token = "", port = DEFAULT_PORT } = await chrome.storage.local.get([
    "token",
    "port",
  ]);
  const line = $("connection");
  try {
    await new WhisprClient(port, token).status();
    line.textContent = "Connected to Whispr";
    line.className = "status good";
    return true;
  } catch (error) {
    line.textContent = error.message;
    line.className = "status bad";
    return false;
  }
}

async function checkMicrophone() {
  const permission = await navigator.permissions
    .query({ name: "microphone" })
    .catch(() => null);
  show("mic-note", permission?.state !== "granted");
}

async function showMain() {
  show("setup", false);
  show("main", true);
  const [tab] = await chrome.tabs.query({ active: true, currentWindow: true });
  $("tab-title").textContent = tab?.title || "";
  $("start").onclick = () => act({ type: "start", tabId: tab?.id });
  await refresh();
  const connected = await checkConnection();
  $("start").disabled = !connected || !tab?.id;
  await checkMicrophone();
}

function showSetup() {
  show("main", false);
  show("setup", true);
  $("token").focus();
}

/** Runs a start or stop, with the buttons off while it runs. */
async function act(message) {
  const buttons = ["start", "stop", "discard"].map($);
  for (const button of buttons) button.disabled = true;
  try {
    render((await background(message)).state);
  } catch (error) {
    showResult({ ok: false, message: error.message });
  } finally {
    for (const button of buttons) button.disabled = false;
  }
}

$("stop").onclick = () => act({ type: "stop" });
$("discard").onclick = () => act({ type: "stop", discard: true });
$("change-token").onclick = showSetup;
$("allow-mic").onclick = () =>
  chrome.tabs.create({ url: chrome.runtime.getURL("permission.html") });
$("save-token").onclick = async () => {
  const token = $("token").value.trim();
  if (!token) return;
  await chrome.storage.local.set({ token });
  $("token").value = "";
  await showMain();
};
$("token").onkeydown = (event) => {
  if (event.key === "Enter") $("save-token").click();
};

const { token } = await chrome.storage.local.get("token");
if (token) {
  await showMain();
} else {
  showSetup();
}
