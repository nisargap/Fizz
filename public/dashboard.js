(() => {
  const $ = (id) => document.getElementById(id);
  const kinds = window.fizzKinds;
  const comparatorText = { gt: 'above', gte: 'at or above', lt: 'below', lte: 'at or below' };
  const statusText = { watching: 'WATCHING', triggered: 'TRIGGERED', paused: 'PAUSED' };
  const freshMs = 15 * 60 * 1000;
  const eventPreview = 5;
  let timer = null;
  let sensors = [];
  let alertData = { rules: [], events: [] };
  let showAllEvents = false;
  let requestInFlight = false;
  let refreshQueued = false;
  let sessionGeneration = 0;

  async function api(path, method = 'GET', body) {
    const response = await fetch(path, { method, credentials: 'same-origin', headers: { 'content-type': 'application/json' }, body: body ? JSON.stringify(body) : undefined });
    const data = await response.json();
    if (!response.ok) throw new Error(data.error?.message || 'Please try again.');
    return data;
  }

  function node(tag, className, content) {
    const element = document.createElement(tag);
    if (className) element.className = className;
    if (content !== undefined) element.textContent = content;
    return element;
  }

  function sensorName(sensor) {
    return sensor ? sensor.name || kinds.get(sensor.kind).label : 'Removed sensor';
  }

  function renderSensors() {
    const list = $('live-sensors');
    list.replaceChildren();
    $('live-empty').hidden = sensors.length > 0;
    for (const sensor of sensors) {
      const info = kinds.get(sensor.kind);
      const card = node('article', 'live-card');
      card.style.setProperty('--sensor-color', info.color);
      const top = node('div', 'live-card-top');
      const title = node('div', 'live-card-title');
      title.append(node('h3', '', sensorName(sensor)), node('span', '', info.label.toUpperCase()));
      const mode = node('span', `mode-pill ${sensor.mode}`, sensor.mode === 'simulated' ? 'SIMULATED' : sensor.mode === 'phone' ? 'PHONE' : 'LIVE API');
      top.append(kinds.badge(sensor.kind), title, mode);
      const latest = sensor.latest_reading;
      const metric = node('p', 'live-metric', latest?.metric ? kinds.metricLabel(latest.metric) : 'Awaiting first reading');
      const value = node('p', 'live-value', kinds.formatReading(latest));
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
      option.textContent = `${sensorName(sensor)} · ${kinds.get(sensor.kind).label}`;
      picker.append(option);
    }
    if (sensors.some((sensor) => sensor.id === prior)) picker.value = prior;
    updateMetricHint();
  }

  function updateMetricHint() {
    const sensor = sensors.find((item) => item.id === $('alert-sensor').value);
    if (sensor && (!$('alert-metric').value || $('alert-metric').dataset.autofill === 'true')) {
      $('alert-metric').value = sensor.latest_reading?.numeric_value != null ? sensor.latest_reading.metric : kinds.get(sensor.kind).metric;
      $('alert-metric').dataset.autofill = 'true';
    }
    if (sensor && (!$('alert-unit').value || $('alert-unit').dataset.autofill === 'true')) {
      $('alert-unit').value = sensor.latest_reading?.numeric_value != null ? sensor.latest_reading.unit : kinds.get(sensor.kind).unit;
      $('alert-unit').dataset.autofill = 'true';
    }
  }

  async function refresh() {
    if ($('dashboard').hidden) return;
    if (requestInFlight) { refreshQueued = true; return; }
    requestInFlight = true;
    const version = sessionGeneration;
    try {
      const [inventory, alerts] = await Promise.all([api('/api/sensors'), api('/api/alerts')]);
      if (version !== sessionGeneration || $('dashboard').hidden) return;
      sensors = inventory.sensors || [];
      alertData = { rules: alerts.rules || [], events: alerts.events || [] };
      renderSensors();
      renderAlerts();
      void window.fizzVoiceClips.refresh(sensors);
    } catch (error) {
      $('dashboard-subtitle').textContent = `Unable to refresh sensor data: ${error.message}`;
    } finally {
      requestInFlight = false;
      if (refreshQueued) { refreshQueued = false; void refresh(); }
    }
  }

  function conditionMet(value, comparator, threshold) {
    if (comparator === 'gt') return value > threshold;
    if (comparator === 'gte') return value >= threshold;
    if (comparator === 'lt') return value < threshold;
    return value <= threshold;
  }

  function limitText(item) {
    return `${comparatorText[item.comparator] || item.comparator} ${kinds.formatValue(item.threshold, item.unit)}`;
  }

  function timeElement(value) {
    const time = node('time', '', kinds.timeAgo(value));
    time.dateTime = new Date(value).toISOString();
    time.title = new Date(value).toLocaleString();
    return time;
  }

  function ruleStatus(rule) {
    if (rule.enabled === false) return 'paused';
    return rule.armed === false ? 'triggered' : 'watching';
  }

  function alertButton(label, className, action, ruleId, handler) {
    const button = node('button', className, label);
    button.type = 'button';
    button.dataset.action = action;
    button.dataset.ruleId = ruleId;
    button.addEventListener('click', async () => {
      button.disabled = true;
      $('alert-message').textContent = '';
      try { await handler(); await refresh(); }
      catch (error) { $('alert-message').textContent = error.message; button.disabled = false; }
    });
    return button;
  }

  function eventItem(event) {
    const sensor = sensors.find((item) => item.id === event.sensor_id);
    const fresh = Date.now() - new Date(event.created_at || event.observed_at).getTime() < freshMs;
    const item = node('li', `alert-item event${fresh ? ' fresh' : ''}`);
    item.style.setProperty('--sensor-color', kinds.get(sensor?.kind).color);
    const body = node('div', 'alert-body');
    const head = node('div', 'alert-item-head');
    head.append(node('strong', '', sensorName(sensor)));
    if (fresh) head.append(node('span', 'alert-new', 'NEW'));
    head.append(timeElement(event.observed_at || event.created_at));
    const line = node('p', 'alert-line', `${kinds.metricLabel(event.metric)} hit `);
    line.append(node('span', 'alert-value', kinds.formatValue(event.numeric_value, event.unit)));
    const meta = node('p', 'alert-meta');
    meta.append(
      node('span', '', `Limit: ${limitText(event)}`),
      node('span', 'over', event.numeric_value === event.threshold ? 'At the limit' : `${kinds.formatValue(Math.abs(event.numeric_value - event.threshold), event.unit)} past the limit`),
      node('span', '', new Date(event.observed_at || event.created_at).toLocaleString([], { dateStyle: 'medium', timeStyle: 'short' })),
    );
    body.append(head, line, meta);
    item.append(kinds.badge(sensor?.kind), body);
    return item;
  }

  function ruleItem(rule) {
    const sensor = sensors.find((item) => item.id === rule.sensor_id);
    const status = ruleStatus(rule);
    const item = node('li', `alert-item rule ${status}`);
    item.style.setProperty('--sensor-color', kinds.get(sensor?.kind).color);
    const body = node('div', 'alert-body');
    const head = node('div', 'alert-item-head');
    head.append(node('strong', '', sensorName(sensor)), node('span', `alert-status ${status}`, statusText[status]));
    const line = node('p', 'alert-line', `Alert when ${kinds.metricLabel(rule.metric)} is ${limitText(rule)}`);
    line.title = rule.metric;
    const meta = node('p', 'alert-meta');
    const latest = sensor?.latest_reading;
    if (latest?.metric === rule.metric && typeof latest.numeric_value === 'number' && (latest.unit || '') === (rule.unit || '')) {
      const now = node('span', '', 'Now ');
      now.append(node('span', 'alert-value', kinds.formatValue(latest.numeric_value, latest.unit)));
      const gap = kinds.formatValue(Math.abs(latest.numeric_value - rule.threshold), rule.unit);
      const met = conditionMet(latest.numeric_value, rule.comparator, rule.threshold);
      const distance = latest.numeric_value === rule.threshold ? 'At the limit' : met ? `${gap} past the limit` : `${gap} from the limit`;
      meta.append(now, node('span', met ? 'over' : '', distance));
    } else if (sensor) {
      meta.append(node('span', '', 'Waiting for a matching reading'));
    }
    const fired = alertData.events.filter((event) => event.rule_id === rule.id);
    if (fired.length) {
      const last = node('span', '', 'Last fired ');
      last.append(timeElement(fired[0].observed_at || fired[0].created_at));
      meta.append(last, node('span', '', `${fired.length}× in recent history`));
    } else {
      meta.append(node('span', '', 'Has not fired yet'));
    }
    if (status === 'triggered') meta.append(node('span', 'over', 'Resets when the reading returns inside the limit'));
    const actions = node('div', 'alert-actions');
    const paused = status === 'paused';
    // Rules on removed sensors cannot be resumed, so they only offer Delete.
    if (sensor) actions.append(alertButton(paused ? 'Resume' : 'Pause', 'secondary', 'toggle', rule.id, () => api('/api/alerts', 'PATCH', { id: rule.id, enabled: paused })));
    actions.append(alertButton('Delete', 'alert-delete', 'delete', rule.id, async () => {
      if (confirm(`Delete the alert for ${sensorName(sensor)}?`)) await api('/api/alerts', 'DELETE', { id: rule.id });
    }));
    body.append(head, line, meta, actions);
    item.append(kinds.badge(sensor?.kind), body);
    return item;
  }

  function renderAlerts() {
    const list = $('dashboard-alerts');
    const focused = document.activeElement?.closest?.('#dashboard-alerts [data-action]');
    const focusKey = focused ? [focused.dataset.action, focused.dataset.ruleId] : null;
    list.replaceChildren();
    const { rules, events } = alertData;
    const counts = { watching: 0, triggered: 0, paused: 0 };
    for (const rule of rules) counts[ruleStatus(rule)]++;
    const summary = $('alert-summary');
    summary.replaceChildren();
    for (const status of ['triggered', 'watching', 'paused']) {
      if (counts[status]) summary.append(node('span', `alert-chip ${status}`, `${counts[status]} ${status.toUpperCase()}`));
    }
    if (!rules.length && !events.length) {
      list.append(node('p', 'dashboard-empty', 'No alerts yet. Set a threshold below, or ask Fizz.'));
      return;
    }
    if (events.length) {
      const group = node('div', 'alert-group');
      group.append(node('h3', '', 'RECENT TRIGGERS'));
      const items = node('ol', 'alert-items');
      for (const event of showAllEvents ? events : events.slice(0, eventPreview)) items.append(eventItem(event));
      group.append(items);
      if (events.length > eventPreview) {
        const more = node('button', 'alert-more', showAllEvents ? 'Show fewer' : `Show all ${events.length} triggers`);
        more.type = 'button';
        more.dataset.action = 'more';
        more.dataset.ruleId = '';
        more.setAttribute('aria-expanded', String(showAllEvents));
        more.addEventListener('click', () => { showAllEvents = !showAllEvents; renderAlerts(); });
        group.append(more);
      }
      list.append(group);
    }
    if (rules.length) {
      const group = node('div', 'alert-group');
      group.append(node('h3', '', 'RULES'));
      const items = node('ul', 'alert-items');
      const rank = { triggered: 0, watching: 1, paused: 2 };
      for (const rule of [...rules].sort((a, b) => rank[ruleStatus(a)] - rank[ruleStatus(b)])) items.append(ruleItem(rule));
      group.append(items);
      list.append(group);
    }
    if (focusKey) list.querySelector(`[data-action="${focusKey[0]}"][data-rule-id="${focusKey[1]}"]`)?.focus();
  }

  function appendMessage(role, message) {
    const item = node('div', `chat-message ${role}`);
    item.append(node('span', '', role === 'agent' ? 'FIZZ' : 'YOU'), node('p', '', message));
    $('chat-messages').append(item);
    $('chat-messages').scrollTop = $('chat-messages').scrollHeight;
    return item;
  }

  function appendThinking() {
    const item = appendMessage('agent', 'Thinking');
    item.classList.add('thinking');
    const dots = node('span', 'thinking-dots');
    for (let i = 0; i < 3; i++) dots.append(node('span', '', '.'));
    item.querySelector('p').append(dots);
    return item;
  }

  function proposalSummary(proposal) {
    const sensor = sensors.find((item) => item.id === proposal.sensor_id);
    const summary = node('p', 'alert-meta', `${sensorName(sensor)} · ${kinds.metricLabel(proposal.metric)} ${limitText(proposal)}`);
    summary.style.setProperty('--sensor-color', kinds.get(sensor?.kind).color);
    return summary;
  }

  function setThinking(active) {
    $('dashboard').classList.toggle('fizz-thinking', active);
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
    const pending = appendThinking();
    setThinking(true);
    const version = sessionGeneration;
    try {
      const result = await api('/api/chat', 'POST', { message });
      if (version !== sessionGeneration || $('dashboard').hidden) return;
      pending.classList.remove('thinking');
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
        pending.append(proposalSummary(result.proposal), confirm);
      }
      $('chat-messages').scrollTop = $('chat-messages').scrollHeight;
    } catch (error) {
      if (version === sessionGeneration) {
        pending.classList.remove('thinking');
        pending.querySelector('p').textContent = 'I could not answer right now.';
        $('chat-error').textContent = error.message;
      }
    }
    finally {
      button.disabled = false;
      if (version === sessionGeneration) { setThinking(false); input.focus(); }
    }
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
      alertData = { rules: [], events: [] };
      showAllEvents = false;
      setThinking(false);
      window.fizzVoiceClips.reset();
      $('chat-messages').replaceChildren();
      appendMessage('agent', 'Hi! Ask me what your sensors are seeing, or tell me what you want to watch for.');
      $('chat-input').value = '';
      $('chat-error').textContent = '';
    },
  };
})();
