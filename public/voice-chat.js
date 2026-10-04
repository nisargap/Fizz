(() => {
  const toggle = document.getElementById('chat-voice-toggle');
  const status = document.getElementById('chat-voice-status');
  const audio = document.getElementById('chat-voice-audio');
  let generation = 0;
  let available = false;
  let busy = false;
  let state = 'idle';
  let recorder = null;
  let stream = null;
  let timeout = null;
  let controller = null;
  let audioUrl = null;
  const supported = !!(window.isSecureContext && navigator.mediaDevices?.getUserMedia && window.MediaRecorder);

  function render() {
    toggle.disabled = !available || !supported || busy || ['permission', 'transcribing', 'thinking', 'loading'].includes(state);
    toggle.textContent = { recording: 'Send recording', speaking: 'Stop reply', permission: 'Opening mic…', transcribing: 'Transcribing…', thinking: 'Thinking…', loading: 'Preparing reply…' }[state] || 'Talk to Fizz';
    toggle.setAttribute('aria-pressed', String(state === 'recording'));
    toggle.classList.toggle('recording', state === 'recording');
  }

  function releaseMic() {
    if (timeout) clearTimeout(timeout);
    timeout = null;
    stream?.getTracks().forEach((track) => track.stop());
    stream = null;
  }

  function stopPlayback() {
    audio.pause();
    if (state === 'speaking') {
      state = 'idle';
      status.textContent = 'Reply stopped. Tap Talk to Fizz to continue.';
      render();
    }
  }

  async function voiceRequest(body, contentType, signal) {
    const response = await fetch('/api/voice', { method: 'POST', credentials: 'same-origin', signal, headers: { 'content-type': contentType }, body });
    if (!response.ok) {
      const data = await response.json();
      throw new Error(data.error?.message || 'Voice chat is temporarily unavailable.');
    }
    return response;
  }

  async function sendRecording(chunks, mime, version) {
    if (version !== generation) return;
    releaseMic();
    recorder = null;
    controller = new AbortController();
    try {
      const recording = new Blob(chunks, { type: mime });
      if (!recording.size) throw new Error('I didn’t hear a recording. Please try again.');
      if (recording.size > 2 * 1024 * 1024) throw new Error('Try a shorter recording, under 30 seconds.');
      state = 'transcribing';
      status.textContent = 'Listening to your message…';
      render();
      const response = await voiceRequest(recording, mime, controller.signal);
      const { text } = await response.json();
      if (version !== generation) return;
      state = 'thinking';
      status.textContent = 'Fizz is answering…';
      render();
      const result = await window.fizzChat.send(text);
      if (version !== generation) return;
      if (!result?.reply) {
        status.textContent = 'Fizz couldn’t answer right now. See the message in chat.';
        return;
      }
      state = 'loading';
      status.textContent = 'Preparing Fizz’s voice…';
      render();
      const speech = await voiceRequest(JSON.stringify({ text: result.reply }), 'application/json', controller.signal);
      const blob = await speech.blob();
      if (version !== generation) return;
      if (audioUrl) URL.revokeObjectURL(audioUrl);
      audioUrl = URL.createObjectURL(blob);
      audio.src = audioUrl;
      audio.hidden = false;
      try { await audio.play(); }
      catch (_) { status.textContent = 'Reply ready. Tap play to hear Fizz.'; }
    } catch (error) {
      if (version === generation && error.name !== 'AbortError') status.textContent = error.message;
    } finally {
      if (version === generation) {
        controller = null;
        if (state !== 'speaking') state = 'idle';
        window.fizzChat.lock(false);
        render();
      }
    }
  }

  async function record() {
    const version = generation;
    stopPlayback();
    state = 'permission';
    window.fizzChat.lock(true);
    status.textContent = 'Allow microphone access to talk to Fizz.';
    render();
    try {
      const capture = await navigator.mediaDevices.getUserMedia({ audio: { echoCancellation: true, noiseSuppression: true }, video: false });
      if (version !== generation) { capture.getTracks().forEach((track) => track.stop()); return; }
      stream = capture;
      const mime = ['audio/webm;codecs=opus', 'audio/ogg;codecs=opus', 'audio/mp4'].find((type) => MediaRecorder.isTypeSupported(type));
      const active = new MediaRecorder(capture, mime ? { mimeType: mime } : undefined);
      recorder = active;
      const chunks = [];
      active.addEventListener('dataavailable', (event) => { if (event.data.size) chunks.push(event.data); });
      active.addEventListener('stop', () => sendRecording(chunks, active.mimeType || 'audio/webm', version), { once: true });
      active.addEventListener('error', () => {
        if (version !== generation) return;
        generation++;
        releaseMic();
        recorder = null;
        state = 'idle';
        status.textContent = 'The microphone stopped. Try again or type your message.';
        window.fizzChat.lock(false);
        render();
      });
      active.start(1000);
      state = 'recording';
      status.textContent = 'Recording · tap Send recording when done · 30-second limit';
      timeout = setTimeout(() => { if (active.state === 'recording') active.stop(); }, 30000);
      render();
    } catch (error) {
      if (version !== generation) return;
      releaseMic();
      recorder = null;
      state = 'idle';
      status.textContent = error.name === 'NotAllowedError' ? 'Microphone access was denied. Allow it in your browser or type to Fizz.' : 'The microphone isn’t available. Try again or type to Fizz.';
      window.fizzChat.lock(false);
      render();
    }
  }

  toggle.addEventListener('click', () => {
    if (state === 'recording') {
      recorder?.stop();
      state = 'transcribing';
      render();
    } else if (state === 'speaking') stopPlayback();
    else if (state === 'idle' && !busy && available) record();
  });
  audio.addEventListener('play', () => {
    state = 'speaking';
    status.textContent = 'Fizz is speaking. Tap Stop reply to interrupt.';
    render();
  });
  audio.addEventListener('pause', () => { if (state === 'speaking') { state = 'idle'; render(); } });
  audio.addEventListener('ended', () => {
    state = 'idle';
    status.textContent = 'Tap Talk to Fizz to continue the conversation.';
    render();
  });

  function reset() {
    generation++;
    controller?.abort();
    controller = null;
    const active = recorder;
    recorder = null;
    if (active?.state === 'recording') active.stop();
    releaseMic();
    audio.pause();
    audio.removeAttribute('src');
    audio.load();
    if (audioUrl) URL.revokeObjectURL(audioUrl);
    audioUrl = null;
    audio.hidden = true;
    available = false;
    busy = false;
    state = 'idle';
    status.textContent = '';
    render();
  }

  window.fizzChatVoice = {
    reset, stopPlayback,
    setBusy(active) { busy = active; render(); },
    async start() {
      reset();
      const version = generation;
      if (!supported) { status.textContent = 'Voice recording isn’t supported in this browser. You can still type to Fizz.'; return; }
      try {
        const response = await fetch('/api/voice', { credentials: 'same-origin', cache: 'no-store' });
        const data = await response.json();
        if (version !== generation) return;
        available = response.ok && data.available === true;
        status.textContent = available ? 'Tap to record. Fizz replies aloud. Audio is transcribed by ElevenLabs.' : 'Voice chat isn’t available yet. You can still type to Fizz.';
      } catch (_) { if (version === generation) status.textContent = 'Voice chat is temporarily unavailable. You can still type to Fizz.'; }
      if (version === generation) render();
    },
  };
  render();
})();
