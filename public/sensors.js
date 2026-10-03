(() => {
  const $ = (id) => document.getElementById(id);
  const labels = { water: 'Water', gas: 'Gas', radio: 'Radio', temperature: 'Temperature', pressure: 'Pressure', humidity: 'Humidity', sound: 'Sound', phone: 'Phone', custom: 'Custom' };
  let sensors = [];
  async function api(path, method = 'GET', body) {
    const response = await fetch(path, { method, credentials: 'same-origin', headers: { 'content-type': 'application/json' }, body: body ? JSON.stringify(body) : undefined });
    const data = await response.json();
    if (!response.ok) throw new Error(data.error?.message || 'Please try again.');
    return data;
  }
  function node(tag, className, content) {
    const el = document.createElement(tag);
    if (className) el.className = className;
    if (content !== undefined) el.textContent = content;
    return el;
  }
  function button(label, className, handler) {
    const el = node('button', className, label);
    el.type = 'button';
    el.addEventListener('click', async () => {
      el.disabled = true;
      try { await handler(); }
      catch (error) { $('page-error').textContent = error.message; }
      finally { el.disabled = false; }
    });
    return el;
  }
  function valueOf(reading) {
    if (!reading) return 'Waiting for data';
    if (reading.metric === 'voice_clip') return 'Clip received';
    return `${reading.numeric_value ?? reading.boolean_value ?? reading.text_value}${reading.unit ? ` ${reading.unit}` : ''}`;
  }
  function showSecret(container, heading, secret) {
    const panel = node('div', 'secret-panel');
    panel.append(node('strong', '', heading), node('p', '', 'Copy this now. It will not be shown again.'));
    const code = node('code', '', secret);
    panel.append(code, button('Copy', 'secondary', async () => { await navigator.clipboard.writeText(secret); }));
    container.replaceChildren(panel);
  }
  function render() {
    $('sensor-count').textContent = `${sensors.length} ${sensors.length === 1 ? 'sensor' : 'sensors'}`;
    $('sensor-empty').hidden = sensors.length > 0;
    const list = $('sensor-list');
    list.replaceChildren();
    for (const sensor of sensors) {
      const card = node('article', 'sensor-manage-card');
      const head = node('div', 'sensor-manage-head');
      const title = node('div');
      title.append(node('span', 'sensor-type', `${labels[sensor.kind] || sensor.kind} / ${sensor.mode === 'simulated' ? 'SIMULATED' : sensor.mode === 'phone' ? 'PHONE' : 'LIVE API'}`), node('h3', '', sensor.name || labels[sensor.kind] || sensor.kind));
      head.append(title, node('span', 'sensor-live-value', valueOf(sensor.latest_reading)));
      const detail = node('p', 'sensor-detail-line', sensor.latest_reading ? `${sensor.latest_reading.metric.replaceAll('_', ' ')} · ${new Date(sensor.latest_reading.observed_at || sensor.latest_reading.received_at).toLocaleString()}` : 'Awaiting first reading');
      const actions = node('div', 'sensor-actions');
      const secret = node('div', 'sensor-secret');
      if (sensor.mode === 'simulated') {
        actions.append(button('Connect API', 'secondary', async () => {
          const result = await api('/api/sensors', 'PATCH', { id: sensor.id, mode: 'api' });
          await refresh();
          if (result.api_key) showSecret(document.querySelector(`[data-sensor-id="${sensor.id}"] .sensor-secret`), 'Sensor API key', result.api_key);
        }));
      } else if (sensor.mode === 'api') {
        actions.append(button('Use sample stream', 'secondary', async () => { await api('/api/sensors', 'PATCH', { id: sensor.id, mode: 'simulated' }); await refresh(); }));
        actions.append(button('Rotate API key', 'secondary', async () => {
          if (!confirm('Rotate this sensor key? The old key will stop working immediately.')) return;
          const result = await api('/api/sensors', 'PATCH', { id: sensor.id, rotate_key: true });
          if (result.api_key) showSecret(secret, 'New sensor API key', result.api_key);
        }));
      }
      if (sensor.kind === 'phone') {
        actions.append(button('Create phone link', 'secondary', async () => {
          const result = await api('/api/phone', 'POST', { action: 'create', sensor_id: sensor.id });
          const url = new URL(result.url, location.origin).href;
          await refresh();
          showSecret(document.querySelector(`[data-sensor-id="${sensor.id}"] .sensor-secret`), 'Send this link to your phone', url);
          await navigator.clipboard.writeText(url).catch(() => {});
        }));
        actions.append(button('Revoke phones', 'secondary', async () => {
          if (!confirm('Disconnect all phones paired with this sensor?')) return;
          await api(`/api/phone?sensor_id=${encodeURIComponent(sensor.id)}`, 'DELETE');
          $('page-error').textContent = 'Phone access revoked.';
        }));
      }
      actions.append(button('Remove sensor', 'sensor-remove', async () => {
        if (!confirm(`Remove ${sensor.name || labels[sensor.kind]}? Its API key and alerts will stop working.`)) return;
        await api('/api/sensors', 'DELETE', { id: sensor.id });
        await refresh();
      }));
      card.dataset.sensorId = sensor.id;
      card.append(head, detail, actions, secret);
      list.append(card);
    }
  }
  async function refresh() {
    const result = await api('/api/sensors');
    sensors = result.sensors || [];
    render();
    void window.fizzVoiceClips.refresh(sensors);
  }
  $('add-sensor-form').addEventListener('submit', async (event) => {
    event.preventDefault();
    const button = $('add-sensor-form').querySelector('button[type=submit]');
    button.disabled = true;
    $('add-message').textContent = '';
    try {
      await api('/api/sensors', 'POST', { kind: $('new-kind').value, name: $('new-name').value.trim() || undefined, mode: 'simulated' });
      $('new-name').value = '';
      $('add-message').textContent = 'Sensor added. Its sample stream is starting.';
      await refresh();
    } catch (error) { $('add-message').textContent = error.message; }
    finally { button.disabled = false; }
  });
  $('sensors-sign-out').addEventListener('click', async () => {
    try { await api('/api/session', 'DELETE'); location.href = '/app.html'; }
    catch (error) { $('page-error').textContent = error.message; }
  });
  (async () => {
    try {
      const me = await api('/api/me');
      $('sensors-username').textContent = me.username;
      await refresh();
      setInterval(() => {
        if (document.hidden) return;
        if (document.querySelector('.sensor-secret code')) void window.fizzVoiceClips.refresh(sensors);
        else refresh().catch(() => {});
      }, 15000);
    } catch { location.href = '/app.html'; }
  })();
})();
