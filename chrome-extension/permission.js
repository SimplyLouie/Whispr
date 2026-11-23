// Chrome only asks for the microphone on a visible page, and the recorder
// page is hidden. Granting it here covers the whole extension.

const outcome = document.getElementById("outcome");
const allow = document.getElementById("allow");

async function request() {
  try {
    const stream = await navigator.mediaDevices.getUserMedia({ audio: true });
    for (const track of stream.getTracks()) track.stop();
    outcome.textContent =
      "The microphone is allowed. You can close this tab and start a recording.";
    outcome.className = "status good";
    allow.hidden = true;
  } catch {
    outcome.textContent =
      "The microphone wasn't allowed. Use the icon in the address bar to allow it, then try again.";
    outcome.className = "status bad";
  }
}

allow.onclick = request;
request();
