(() => {
  const $ = (id) => document.getElementById(id);
  const labels = { water: 'Water', gas: 'Gas', radio: 'Radio', temperature: 'Temperature', pressure: 'Pressure', humidity: 'Humidity', sound: 'Sound', phone: 'Phone', custom: 'Custom' };
  const defaultMetric = { water: 'flow_l_min', gas: 'concentration_ppm', radio: 'rssi_dbm', temperature: 'temperature_c', pressure: 'pressure_kpa', humidity: 'humidity_pct', sound: 'sound_db', phone: 'acceleration_ms2', custom: 'value' };
  const defaultUnit = { water: 'L/min', gas: 'ppm', radio: 'dBm', temperature: '°C', pressure: 'kPa', humidity: '%', sound: 'dB', phone: 'm/s²', custom: '' };
  let timer = null;
  let sensors = [];
  let requestInFlight = false;
  let sessionGeneration = 0;

  async function api(path, method = 'GET', body) {
    const response = await fetch(path, { method, credentials: 'same-origin', headers: { 'content-type': 'application/json' }, body: body ? JSON.stringify(body) : undefined });
    const data = await response.json();
    if (!response.ok) throw new Error(data.error?.message || 'Please try again.');
    return data;
  }

  function valueOf(reading) {
    if (!reading) return 'Waiting for data';
    if (reading.metric === 'voice_clip') return 'Clip received';
    const value = reading.numeric_value ?? reading.boolean_value ?? reading.text_value;
    return `${value}${reading.unit ? ` ${reading.unit}` : ''}`;
  }

  function node(tag, className, content) {
    const element = document.createElement(tag);
    if (className) element.className = className;
    if (content !== undefined) element.textContent = content;
    return element;
  }

  function renderSensors() {
    const list = $('live-sensors');
    list.replaceChildren();
    $('live-empty').hidden = sensors.length > 0;
    for (const sensor of sensors) {
      const card = node('article', 'live-card');
      const top = node('div', 'live-card-top');
      const icon = node('span', 'live-icon', (labels[sensor.kind] || 'S').slice(0, 1));
      icon.setAttribute('aria-hidden', 'true');
      const title = node('div', 'live-card-title');
      title.append(node('h3', '', sensor.name || labels[sensor.kind] || sensor.kind), node('span', '', labels[sensor.kind] || sensor.kind));
      const mode = node('span', `mode-pill ${sensor.mode}`, sensor.mode === 'simulated' ? 'SIMULATED' : sensor.mode === 'phone' ? 'PHONE' : 'LIVE API');
      top.append(icon, title, mode);
      const latest = sensor.latest_reading;
      const metric = node('p', 'live-metric', latest?.metric ? latest.metric.replaceAll('_', ' ') : 'Awaiting first reading');
      const value = node('p', 'live-value', valueOf(latest));
      const foot = node('p', 'live-foot', latest ? `Updated ${new Date(latest.observed_at || latest.received_at).toLocaleTimeString()}` : 'A sample will appear shortly');
      card.append(top, metric, value, foot);
      list.append(card);
    }
    const picker = $('alert-sensor');
    const prior = picker.value;
    picker.replaceChildren();
    for (const sensor of sensors) {
      const option = document.createElement('option');
      option.value = sensor.id;
      option.textContent = sensor.name || labels[sensor.kind];
      picker.append(option);
    }
    if (sensors.some((sensor) => sensor.id === prior)) picker.value = prior;
    updateMetricHint();
  }

  function updateMetricHint() {
    const sensor = sensors.find((item) => item.id === $('alert-sensor').value);
    if (sensor && (!$('alert-metric').value || $('alert-metric').dataset.autofill === 'true')) {
      $('alert-metric').value = sensor.latest_reading?.numeric_value != null ? sensor.latest_reading.metric : defaultMetric[sensor.kind] || 'value';
      $('alert-metric').dataset.autofill = 'true';
    }
    if (sensor && (!$('alert-unit').value || $('alert-unit').dataset.autofill === 'true')) {
      $('alert-unit').value = sensor.latest_reading?.numeric_value != null ? sensor.latest_reading.unit : defaultUnit[sensor.kind] || 'unit';
      $('alert-unit').dataset.autofill = 'true';
    }
  }

  async function refresh() {
    if (requestInFlight || $('dashboard').hidden) return;
    requestInFlight = true;
    const version = sessionGeneration;
    try {
      const [inventory, alerts] = await Promise.all([api('/api/sensors'), api('/api/alerts')]);
      if (version !== sessionGeneration || $('dashboard').hidden) return;
      sensors = inventory.sensors || [];
      renderSensors();
      renderAlerts(alerts);
      void window.fizzVoiceClips.refresh(sensors);
    } catch (error) {
      $('dashboard-subtitle').textContent = `Unable to refresh sensor data: ${error.message}`;
    } finally { requestInFlight = false; }
  }

  function renderAlerts(data) {
    const list = $('dashboard-alerts');
    list.replaceChildren();
    const rules = data.rules || [];
    const events = data.events || [];
    if (!rules.length && !events.length) {
      list.append(node('p', 'dashboard-empty', 'No alerts yet. Set a threshold below, or ask Fizz.'));
      return;
    }
    for (const event of events.slice(0, 4)) {
      const row = node('div', 'alert-row triggered');
      row.append(node('span', 'alert-dot', '✦'), node('span', '', event.message || `${event.metric || 'Sensor'} crossed an alert threshold`));
      list.append(row);
    }
    for (const rule of rules.filter((item) => item.enabled !== false).slice(0, 5)) {
      const sensor = sensors.find((item) => item.id === rule.sensor_id);
      const row = node('div', 'alert-row');
      row.append(node('span', 'alert-dot', '◇'), node('span', '', `${sensor?.name || 'Sensor'}: ${rule.metric} ${rule.comparator} ${rule.threshold}${rule.unit ? ` ${rule.unit}` : ''}`));
      list.append(row);
    }
  }

  function appendMessage(role, message) {
    const item = node('div', `chat-message ${role}`);
    item.append(node('span', '', role === 'agent' ? 'FIZZ' : 'YOU'), node('p', '', message));
    $('chat-messages').append(item);
    $('chat-messages').scrollTop = $('chat-messages').scrollHeight;
    return item;
  }

  $('alert-sensor').addEventListener('change', updateMetricHint);
  $('alert-metric').addEventListener('input', () => { $('alert-metric').dataset.autofill = 'false'; });
  $('alert-unit').addEventListener('input', () => { $('alert-unit').dataset.autofill = 'false'; });
  $('quick-alert').addEventListener('submit', async (event) => {
    event.preventDefault();
    $('alert-message').textContent = '';
    const button = $('quick-alert').querySelector('button[type=submit]');
    button.disabled = true;
    try {
      await api('/api/alerts', 'POST', { sensor_id: $('alert-sensor').value, metric: $('alert-metric').value.trim(), comparator: $('alert-operator').value, threshold: Number($('alert-threshold').value), unit: $('alert-unit').value.trim() });
      $('alert-message').textContent = 'Alert is active.';
      $('alert-threshold').value = '';
      await refresh();
    } catch (error) { $('alert-message').textContent = error.message; }
    finally { button.disabled = false; }
  });
  $('chat-form').addEventListener('submit', async (event) => {
    event.preventDefault();
    const input = $('chat-input');
    const message = input.value.trim();
    if (!message) return;
    input.value = '';
    $('chat-error').textContent = '';
    appendMessage('user', message);
    const button = $('chat-form').querySelector('button[type=submit]');
    button.disabled = true;
    const pending = appendMessage('agent', 'Thinking...');
    const version = sessionGeneration;
    try {
      const result = await api('/api/chat', 'POST', { message });
      if (version !== sessionGeneration || $('dashboard').hidden) return;
      pending.querySelector('p').textContent = result.reply || 'I could not find an answer.';
      window.fizzVoiceClips.attach(pending, result.clips);
      if (result.proposal) {
        const confirm = node('button', 'proposal-button', 'Activate this alert');
        confirm.type = 'button';
        confirm.addEventListener('click', async () => {
          confirm.disabled = true;
          try {
            await api('/api/alerts', 'POST', result.proposal);
            confirm.textContent = 'Alert active ✓';
            await refresh();
          } catch (error) { $('chat-error').textContent = error.message; confirm.disabled = false; }
        });
        pending.append(confirm);
      }
      $('chat-messages').scrollTop = $('chat-messages').scrollHeight;
    } catch (error) {
      if (version === sessionGeneration) { pending.querySelector('p').textContent = 'I could not answer right now.'; $('chat-error').textContent = error.message; }
    }
    finally { button.disabled = false; if (version === sessionGeneration) input.focus(); }
  });

  window.fizzDashboard = {
    start(username, initialSensors) {
      sessionGeneration++;
      $('dashboard-username').textContent = username;
      $('dashboard-subtitle').textContent = 'Your sensors are ready.';
      sensors = initialSensors || [];
      renderSensors();
      if (timer) clearInterval(timer);
      refresh();
      timer = setInterval(refresh, 5000);
    },
    stop() {
      sessionGeneration++;
      if (timer) clearInterval(timer);
      timer = null;
      sensors = [];
      window.fizzVoiceClips.reset();
      $('chat-messages').replaceChildren();
      appendMessage('agent', 'Hi! Ask me what your sensors are seeing, or tell me what you want to watch for.');
      $('chat-input').value = '';
      $('chat-error').textContent = '';
    },
  };
})();
