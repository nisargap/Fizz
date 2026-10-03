const $ = (id) => document.getElementById(id);
const storageKey = 'fizz_phone_device';
let deviceToken = localStorage.getItem(storageKey);
let motionActive = false;
let lastMotionSent = 0;
let lastOrientationSent = 0;
let locationWatch = null;
let lastLocationSent = 0;
let stream;

function message(title, detail, retry = false) {
  $('connection-title').textContent = title;
  $('connection-detail').textContent = detail;
  $('retry').hidden = !retry;
}

async function request(action, data = {}, useDevice = false) {
  const response = await fetch('/api/phone', {
    method: 'POST', headers: { 'content-type': 'application/json', ...(useDevice ? { authorization: `Bearer ${deviceToken}` } : {}) },
    body: JSON.stringify({ action, ...data }), cache: 'no-store',
  });
  const result = await response.json().catch(() => ({}));
  if (!response.ok) throw new Error(result.error?.message || 'Fizz could not complete this request.');
  return result;
}

async function connect() {
  const fragment = new URLSearchParams(location.hash.slice(1));
  const pairingToken = fragment.get('pair');
  if (pairingToken) {
    history.replaceState(null, '', location.pathname);
    message('Connecting your phone…', 'The link can only be used once.');
    try {
      const result = await request('claim', { token: pairingToken });
      deviceToken = result.device_token;
      localStorage.setItem(storageKey, deviceToken);
    } catch (error) {
      message('This link cannot connect.', `${error.message} Ask the account owner for a new pairing link.`, true);
      return;
    }
  }
  if (!deviceToken) {
    message('A pairing link is needed.', 'Ask the Fizz account owner to create a Phone sensor and send you its unique link.', true);
    return;
  }
  message('Phone connected', 'Choose which signals to share below.');
  $('sensor-section').hidden = false;
}

function eventId() { return `${Date.now()}-${crypto.randomUUID()}`; }

async function sendMetric(metric, value) {
  if (!Number.isFinite(value)) return;
  try { await request('reading', { event_id: eventId(), metric, value: Math.round(value * 1000) / 1000 }, true); }
  catch (error) {
    if (/no longer active|Connect this phone/.test(error.message)) disconnect(error.message);
    else $('motion-status').textContent = `Stream paused: ${error.message}`;
  }
}

function onMotion(event) {
  if (!motionActive || Date.now() - lastMotionSent < 1000) return;
  const a = event.accelerationIncludingGravity || event.acceleration;
  if (!a || [a.x, a.y, a.z].every((v) => v == null)) return;
  lastMotionSent = Date.now();
  const values = [a.x, a.y, a.z].map(Number);
  $('accel-readout').textContent = Math.hypot(...values.filter(Number.isFinite)).toFixed(1);
  ['acceleration_x', 'acceleration_y', 'acceleration_z'].forEach((metric, i) => {
    if (Number.isFinite(values[i])) void sendMetric(metric, values[i]);
  });
  $('motion-status').textContent = 'Live motion data is streaming.';
}

function onOrientation(event) {
  if (!motionActive || Date.now() - lastOrientationSent < 1000) return;
  if ([event.alpha, event.beta, event.gamma].every((v) => v == null)) return;
  lastOrientationSent = Date.now();
  ['rotation_alpha', 'rotation_beta', 'rotation_gamma'].forEach((metric, i) => {
    const value = [event.alpha, event.beta, event.gamma][i];
    if (typeof value === 'number') void sendMetric(metric, value);
  });
}

function stopMotion() {
  motionActive = false;
  window.removeEventListener('devicemotion', onMotion);
  window.removeEventListener('deviceorientation', onOrientation);
  $('motion-button').textContent = 'Start motion';
  $('motion-status').textContent = 'Motion sharing is off.';
}

function stopLocation() {
  if (locationWatch !== null) navigator.geolocation.clearWatch(locationWatch);
  locationWatch = null;
  $('location-button').textContent = 'Start location';
  $('location-status').textContent = 'Location is off.';
}

function toggleLocation() {
  if (locationWatch !== null) { stopLocation(); return; }
  if (!isSecureContext || !navigator.geolocation) {
    $('location-status').textContent = 'Location is unavailable in this browser. Use HTTPS.';
    return;
  }
  $('location-status').textContent = 'Requesting location permission…';
  locationWatch = navigator.geolocation.watchPosition((position) => {
    $('location-button').textContent = 'Stop location';
    $('location-status').textContent = `Sharing location (±${Math.round(position.coords.accuracy)} m).`;
    if (Date.now() - lastLocationSent < 10000) return;
    lastLocationSent = Date.now();
    void sendMetric('latitude', position.coords.latitude);
    void sendMetric('longitude', position.coords.longitude);
    void sendMetric('location_accuracy', position.coords.accuracy);
  }, (error) => {
    stopLocation();
    $('location-status').textContent = error.code === 1 ? 'Location permission was declined.' : 'Location is unavailable right now.';
  }, { enableHighAccuracy: false, maximumAge: 10000, timeout: 15000 });
}

async function toggleMotion() {
  if (motionActive) { stopMotion(); return; }
  try {
    if (!isSecureContext) throw new Error('Motion needs HTTPS.');
    if (!('DeviceMotionEvent' in window) && !('DeviceOrientationEvent' in window)) throw new Error('This browser does not expose motion sensors.');
    for (const type of [window.DeviceMotionEvent, window.DeviceOrientationEvent]) {
      if (typeof type?.requestPermission === 'function' && await type.requestPermission() !== 'granted') throw new Error('Motion permission was declined.');
    }
    motionActive = true;
    window.addEventListener('devicemotion', onMotion);
    window.addEventListener('deviceorientation', onOrientation);
    $('motion-button').textContent = 'Stop motion';
    $('motion-status').textContent = 'Waiting for this phone to send motion…';
  } catch (error) { $('motion-status').textContent = error.message; }
}

async function recordClip() {
  const button = $('voice-button');
  button.disabled = true;
  $('voice-status').textContent = 'Requesting microphone permission…';
  try {
    if (!isSecureContext || !navigator.mediaDevices?.getUserMedia || !window.MediaRecorder) throw new Error('This browser cannot record audio here. Use HTTPS and a supported browser.');
    stream = await navigator.mediaDevices.getUserMedia({ audio: true });
    const mime = ['audio/webm;codecs=opus', 'audio/webm', 'audio/mp4', 'audio/ogg;codecs=opus'].find((candidate) => MediaRecorder.isTypeSupported(candidate));
    if (!mime) throw new Error('This browser has no supported audio format.');
    const chunks = [];
    const recorder = new MediaRecorder(stream, { mimeType: mime });
    const recorded = new Promise((resolve, reject) => {
      recorder.ondataavailable = (event) => { if (event.data.size) chunks.push(event.data); };
      recorder.onerror = () => reject(new Error('Recording failed.'));
      recorder.onstop = () => resolve(new Blob(chunks, { type: mime }));
    });
    recorder.start();
    $('voice-status').textContent = 'Recording for 3 seconds…';
    await new Promise((resolve) => setTimeout(resolve, 3000));
    recorder.stop();
    const blob = await recorded;
    stream.getTracks().forEach((track) => track.stop());
    stream = null;
    if (blob.size > 262144) throw new Error('Clip is too large. Try a browser with a smaller audio format.');
    const encoded = await new Promise((resolve, reject) => {
      const reader = new FileReader();
      reader.onload = () => resolve(String(reader.result).split(',')[1]);
      reader.onerror = () => reject(new Error('Could not read the clip.'));
      reader.readAsDataURL(blob);
    });
    $('voice-status').textContent = 'Sending clip to Fizz…';
    await request('clip', { mime_type: mime, audio_base64: encoded }, true);
    $('voice-status').textContent = 'Clip sent. Microphone is off.';
  } catch (error) { $('voice-status').textContent = error.message; }
  finally { stream?.getTracks().forEach((track) => track.stop()); stream = null; button.disabled = false; }
}

async function sendTranscript() {
  const words = $('transcript').value.trim();
  if (!words) { $('transcript-status').textContent = 'Enter or dictate a few words first.'; return; }
  $('transcript-button').disabled = true;
  try {
    await request('transcript', { event_id: eventId(), text: words }, true);
    $('transcript').value = '';
    $('transcript-status').textContent = 'Words sent. Fizz can query them in chat.';
  } catch (error) { $('transcript-status').textContent = error.message; }
  finally { $('transcript-button').disabled = false; }
}

function dictate() {
  const Recognition = window.SpeechRecognition || window.webkitSpeechRecognition;
  if (!Recognition) {
    $('transcript-status').textContent = 'Dictation is unavailable in this browser. Type the words above instead.';
    return;
  }
  const recognition = new Recognition();
  recognition.lang = navigator.language || 'en-US';
  recognition.interimResults = false;
  recognition.maxAlternatives = 1;
  recognition.onresult = (event) => {
    const words = event.results[0]?.[0]?.transcript || '';
    $('transcript').value = `${$('transcript').value} ${words}`.trim().slice(0, 500);
    $('transcript-status').textContent = 'Review the words, then tap Send words.';
  };
  recognition.onerror = () => { $('transcript-status').textContent = 'Dictation failed. You can type the words instead.'; };
  $('transcript-status').textContent = 'Listening for words…';
  recognition.start();
}

function disconnect(detail = 'This phone is disconnected. Ask for a new pairing link to reconnect.') {
  stopMotion();
  stopLocation();
  localStorage.removeItem(storageKey);
  deviceToken = null;
  $('sensor-section').hidden = true;
  message('Phone disconnected', detail, true);
}

$('motion-button').addEventListener('click', toggleMotion);
$('location-button').addEventListener('click', toggleLocation);
$('voice-button').addEventListener('click', recordClip);
$('dictate-button').addEventListener('click', dictate);
$('transcript-button').addEventListener('click', sendTranscript);
$('disconnect').addEventListener('click', async () => {
  try { await request('disconnect', {}, true); } catch { /* Local access is cleared even when offline. */ }
  disconnect();
});
$('retry').addEventListener('click', () => location.reload());
void connect();
