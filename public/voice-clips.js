(() => {
  const panel = document.getElementById('voice-clips-panel');
  const list = document.getElementById('voice-clips-list');
  const status = document.getElementById('voice-clips-status');
  const rows = new Map();
  let generation = 0;
  let pending = null;

  function node(tag, className, text) {
    const element = document.createElement(tag);
    if (className) element.className = className;
    if (text !== undefined) element.textContent = text;
    return element;
  }

  async function api(path) {
    const response = await fetch(path, { credentials: 'same-origin', cache: 'no-store' });
    const data = await response.json();
    if (!response.ok) throw new Error(data.error?.message || 'Voice clips are unavailable. Please try again.');
    return data;
  }

  function createRow(clip) {
    const element = node('article', 'voice-clip');
    element.dataset.clipId = clip.clip_id;
    const heading = node('div', 'voice-clip-heading');
    const name = node('h3');
    const time = node('time');
    const play = node('button', 'secondary voice-clip-play', '▶ Play clip');
    play.type = 'button';
    const audio = node('audio');
    audio.controls = true;
    audio.preload = 'none';
    audio.hidden = true;
    const message = node('p', 'voice-clip-message');
    message.setAttribute('role', 'status');
    let objectUrl = null;
    let disposed = false;

    function update(metadata) {
      name.textContent = metadata.sensor_name || 'Phone';
      time.dateTime = metadata.created_at;
      time.textContent = new Date(metadata.created_at).toLocaleString();
      audio.setAttribute('aria-label', `Voice clip from ${name.textContent}, ${time.textContent}`);
      play.setAttribute('aria-label', `Play voice clip from ${name.textContent}, ${time.textContent}`);
    }
    update(clip);
    heading.append(name, time);
    element.append(heading, play, audio, message);

    play.addEventListener('click', async () => {
      play.disabled = true;
      message.textContent = 'Loading clip…';
      try {
        const data = await api(`/api/phone?clip_id=${encodeURIComponent(clip.clip_id)}`);
        if (disposed) return;
        const binary = atob(data.audio_base64);
        const bytes = Uint8Array.from(binary, (character) => character.charCodeAt(0));
        if (objectUrl) URL.revokeObjectURL(objectUrl);
        objectUrl = URL.createObjectURL(new Blob([bytes], { type: data.mime_type }));
        audio.src = objectUrl;
        audio.hidden = false;
        play.hidden = true;
        message.textContent = '';
        try { await audio.play(); }
        catch (error) {
          if (!disposed) message.textContent = error.name === 'NotAllowedError'
            ? 'Clip ready. Tap play in the audio controls.'
            : 'This browser could not play the clip. Try another browser.';
        }
      } catch (error) {
        if (!disposed) message.textContent = error.message;
      } finally { play.disabled = false; }
    });
    audio.addEventListener('play', () => {
      message.textContent = '';
      for (const row of rows.values()) if (row.audio !== audio) row.audio.pause();
    });
    audio.addEventListener('error', () => {
      if (!disposed) message.textContent = 'This browser could not play the clip. Try another browser.';
    });

    return {
      element, audio, sensorId: clip.sensor_id, update,
      dispose() {
        disposed = true;
        audio.pause();
        audio.removeAttribute('src');
        audio.load();
        if (objectUrl) URL.revokeObjectURL(objectUrl);
        element.remove();
      },
    };
  }

  function reset() {
    generation++;
    pending = null;
    for (const row of rows.values()) row.dispose();
    rows.clear();
    panel.hidden = true;
    status.textContent = '';
  }

  window.fizzVoiceClips = {
    reset,
    async refresh(sensors) {
      if (!sensors.some((sensor) => sensor.kind === 'phone')) { reset(); return; }
      panel.hidden = false;
      if (pending) return pending;
      const version = generation;
      pending = (async () => {
        try {
          const data = await api('/api/phone?clips=1');
          if (version !== generation) return;
          const clips = data.clips || [];
          const current = new Set(clips.map((clip) => clip.clip_id));
          const sensorIds = new Set(sensors.map((sensor) => sensor.id));
          for (const [id, row] of rows) {
            // Keep playing audio mounted while live readings and clip metadata refresh.
            if (!sensorIds.has(row.sensorId) || (!current.has(id) && row.audio.paused)) {
              row.dispose();
              rows.delete(id);
            }
          }
          for (let i = clips.length - 1; i >= 0; i--) {
            const clip = clips[i];
            if (rows.has(clip.clip_id)) { rows.get(clip.clip_id).update(clip); continue; }
            const row = createRow(clip);
            rows.set(clip.clip_id, row);
            list.prepend(row.element);
          }
          status.textContent = clips.length ? 'Recent clips · tap to listen' : 'No voice clips yet. Record one on your connected phone.';
        } catch (error) {
          if (version === generation) status.textContent = error.message;
        } finally { if (version === generation) pending = null; }
      })();
      return pending;
    },
  };
})();
