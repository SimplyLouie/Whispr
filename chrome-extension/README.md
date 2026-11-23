# Whispr Meeting Capture

A Chrome extension that records a meeting in a browser tab (Google Meet, Teams
or Zoom on the web) and sends it to the Whispr app on the same computer. Whispr
transcribes it locally when the meeting ends and puts the transcript in
History, with what you said marked `Me` and everyone else marked `Others`.

Nothing leaves the computer: the extension only talks to Whispr on `127.0.0.1`.

## Set up

1. In Whispr, open **Settings > Advanced**, turn on **Meeting Capture**, and copy
   the **Pairing Token**.
2. In Chrome, open `chrome://extensions`, turn on **Developer mode**, choose
   **Load unpacked**, and pick this `chrome-extension` folder.
3. Click the extension's icon, paste the pairing token, and save.
4. Click **Allow the microphone** in the popup once, so your own voice is
   recorded too. Without it only the other participants are transcribed.

## Record a meeting

1. Open the meeting tab and click the extension's icon.
2. Click **Record this tab**. The icon shows `REC` while it records.
3. When the meeting is over, click the icon and **Stop and transcribe**. Closing
   the meeting tab ends the recording the same way.

The transcript appears in Whispr's History once it has been transcribed, which
takes a while for a long meeting. **Discard** throws the recording away instead.

## Good to know

- Tell the people in the meeting that it is being recorded.
- Your microphone is recorded for the whole meeting, including while you are
  muted in the meeting itself.
- Use headphones if you can. On speakers your microphone also hears the other
  participants, and some of what they say can show up under `Me`.
- Whispr has to be running with Meeting Capture on. If it stops answering, the
  extension keeps the audio and retries; Whispr saves what it received if the
  browser or the extension goes away.
- Only browser tabs can be recorded, not the Zoom or Teams desktop apps.

## How it works

The popup starts a hidden recorder page (`offscreen.js`) that captures the tab
and the microphone, resamples both to 16 kHz, and posts them every few seconds
to Whispr's endpoint on port 47821 as two-channel audio: microphone on the left,
tab on the right. The protocol is described in
`src-tauri/src/meeting/server.rs`.
