(() => {
  const $ = (id) => document.getElementById(id);
  const kinds = window.fizzKinds;
  let sensors = [];
  let activeKind = 0;
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
      const info = kinds.get(sensor.kind);
      const card = node('article', 'sensor-manage-card');
      card.style.setProperty('--sensor-color', info.color);
      const head = node('div', 'sensor-manage-head');
      const title = node('div', 'sensor-manage-title');
      title.append(node('span', 'sensor-type', `${info.label.toUpperCase()} / ${sensor.mode === 'simulated' ? 'SIMULATED' : sensor.mode === 'phone' ? 'PHONE' : 'LIVE API'}`), node('h3', '', sensor.name || info.label));
      head.append(kinds.badge(sensor.kind, 'kind-badge sensor-manage-badge'), title, node('span', 'sensor-live-value', kinds.formatReading(sensor.latest_reading)));
      const detail = node('p', 'sensor-detail-line', sensor.latest_reading ? `${kinds.metricLabel(sensor.latest_reading.metric)} · ${new Date(sensor.latest_reading.observed_at || sensor.latest_reading.received_at).toLocaleString()}` : 'Awaiting first reading');
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
        if (!confirm(`Remove ${sensor.name || info.label}? Its API key and alerts will stop working.`)) return;
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
  function kindText(kind) {
    const info = kinds.get(kind);
    const text = node('span', 'kind-text');
    text.append(node('span', 'kind-name', info.label), node('span', 'kind-detail', info.detail));
    return text;
  }
  function renderKindButton() {
    const kind = $('new-kind').value;
    const info = kinds.get(kind);
    $('kind-button').style.setProperty('--sensor-color', info.color);
    const caret = node('span', 'kind-caret', '▾');
    caret.setAttribute('aria-hidden', 'true');
    $('kind-button').replaceChildren(kinds.badge(kind), kindText(kind), caret);
    $('new-name').placeholder = `e.g. ${info.example}`;
  }
  function setActiveKind(index) {
    const options = [...$('kind-options').children];
    activeKind = (index + options.length) % options.length;
    options.forEach((option, i) => option.classList.toggle('active', i === activeKind));
    $('kind-options').setAttribute('aria-activedescendant', options[activeKind].id);
    options[activeKind].scrollIntoView({ block: 'nearest' });
  }
  function openKindPicker() {
    $('kind-options').hidden = false;
    $('kind-button').setAttribute('aria-expanded', 'true');
    for (const option of $('kind-options').children) option.setAttribute('aria-selected', String(option.dataset.kind === $('new-kind').value));
    setActiveKind(kinds.order.indexOf($('new-kind').value));
    $('kind-options').focus();
  }
  function closeKindPicker(returnFocus = true) {
    if ($('kind-options').hidden) return;
    $('kind-options').hidden = true;
    $('kind-button').setAttribute('aria-expanded', 'false');
    if (returnFocus) $('kind-button').focus();
  }
  function chooseKind(kind) {
    $('new-kind').value = kind;
    renderKindButton();
    closeKindPicker();
  }
  for (const kind of kinds.order) {
    const info = kinds.get(kind);
    const option = node('li', 'kind-option');
    option.id = `kind-option-${kind}`;
    option.dataset.kind = kind;
    option.setAttribute('role', 'option');
    option.style.setProperty('--sensor-color', info.color);
    const text = kindText(kind);
    text.append(node('span', 'kind-metric', info.unit ? `${info.metric} · ${info.unit}` : info.metric));
    option.append(kinds.badge(kind), text);
    option.addEventListener('click', () => chooseKind(kind));
    option.addEventListener('pointermove', () => { if (activeKind !== kinds.order.indexOf(kind)) setActiveKind(kinds.order.indexOf(kind)); });
    $('kind-options').append(option);
  }
  // Keep focus in the open list so its focusout does not close it before the toggle click lands.
  $('kind-button').addEventListener('mousedown', (event) => { if (!$('kind-options').hidden) event.preventDefault(); });
  $('kind-button').addEventListener('click', () => { if ($('kind-options').hidden) openKindPicker(); else closeKindPicker(); });
  $('kind-button').addEventListener('keydown', (event) => {
    if (event.key !== 'ArrowDown' && event.key !== 'ArrowUp') return;
    event.preventDefault();
    openKindPicker();
  });
  $('kind-options').addEventListener('keydown', (event) => {
    const last = kinds.order.length - 1;
    if (event.key === 'ArrowDown') setActiveKind(activeKind + 1);
    else if (event.key === 'ArrowUp') setActiveKind(activeKind - 1);
    else if (event.key === 'Home') setActiveKind(0);
    else if (event.key === 'End') setActiveKind(last);
    else if (event.key === 'Enter' || event.key === ' ') chooseKind(kinds.order[activeKind]);
    else if (event.key === 'Escape') closeKindPicker();
    else if (event.key === 'Tab') { closeKindPicker(false); return; }
    else if (/^[a-z]$/i.test(event.key)) {
      // Type-ahead: jump to the next sensor type starting with the pressed letter.
      const next = [...kinds.order.keys()].map((i) => (activeKind + 1 + i) % kinds.order.length)
        .find((i) => kinds.get(kinds.order[i]).label.toLowerCase().startsWith(event.key.toLowerCase()));
      if (next !== undefined) setActiveKind(next);
    } else return;
    event.preventDefault();
  });
  $('kind-options').addEventListener('focusout', (event) => {
    if (!$('kind-picker').contains(event.relatedTarget)) closeKindPicker(false);
  });
  renderKindButton();
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
