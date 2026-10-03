const $ = (id) => document.getElementById(id);
const form = $('account-form');
let mode = 'create';
let step = 1;
const sensorNames = {
  water: 'Water', gas: 'Gas', radio: 'Radio', temperature: 'Temperature',
  pressure: 'Pressure', humidity: 'Humidity', sound: 'Sound', phone: 'Phone', custom: 'Custom',
};
let chosenKinds = [];
let signedInUsername = '';

function showError(message) {
  $('form-error').textContent = message;
  $('form-error').hidden = !message;
}

function render() {
  const creating = mode === 'create';
  $('create-mode').classList.toggle('selected', creating);
  $('create-mode').setAttribute('aria-pressed', String(creating));
  $('sign-in-mode').classList.toggle('selected', !creating);
  $('sign-in-mode').setAttribute('aria-pressed', String(!creating));
  $('username-step').hidden = creating && step === 2;
  $('code-step').hidden = creating && step === 1;
  $('chosen-username').textContent = $('username').value.trim().toLowerCase();
  $('confirm-wrap').hidden = !creating;
  $('edit-username').hidden = !creating;
  $('back').hidden = !creating || step === 1;
  $('step-label').textContent = creating ? `${String(step).padStart(2, '0')} / 02` : 'SIGN IN';
  $('auth-title').textContent = creating ? (step === 1 ? 'Create your access' : 'Set your access code') : 'Welcome back';
  $('auth-intro').textContent = creating
    ? (step === 1 ? 'Start with a unique username for your Fizz workspace.' : 'Choose a six digit code to protect your account.')
    : 'Enter your username and six digit access code.';
  $('continue').innerHTML = creating && step === 1 ? 'Continue <span aria-hidden="true">→</span>' : creating ? 'Create account' : 'Sign in';
  $('code').autocomplete = creating ? 'new-password' : 'current-password';
  showError('');
}

function setMode(next) {
  mode = next;
  step = 1;
  form.reset();
  render();
  $('username').focus();
}

function validUsername() {
  const name = $('username').value.trim();
  if (!/^[A-Za-z][A-Za-z0-9_]{2,23}$/.test(name)) {
    showError('Use 3–24 letters, numbers, or underscores. Start with a letter.');
    $('username').focus();
    return false;
  }
  return true;
}

function validCode() {
  if (!/^[0-9]{6}$/.test($('code').value)) {
    showError('Enter exactly six digits for your access code.');
    $('code').focus();
    return false;
  }
  if (mode === 'create' && $('code').value !== $('confirm-code').value) {
    showError('The access codes do not match.');
    $('confirm-code').focus();
    return false;
  }
  return true;
}

function setupError(message) {
  $('setup-error').textContent = message;
  $('setup-error').hidden = !message;
}

function renderSetup() {
  $('setup-loading').hidden = true;
  $('sensor-question').hidden = false;
  $('setup-retry').hidden = true;
  document.querySelectorAll('.sensor-card').forEach((card) => {
    card.setAttribute('aria-pressed', String(chosenKinds.includes(card.dataset.kind)));
  });
  const count = chosenKinds.length;
  $('setup-start').disabled = count === 0;
  $('selection-status').textContent = count === 0
    ? 'No sensors selected yet'
    : `${count} sensor ${count === 1 ? 'type' : 'types'} selected: ${chosenKinds.map((kind) => sensorNames[kind]).join(', ')}`;
}

async function loadSetup() {
  $('sensor-question').hidden = true;
  $('setup-loading').hidden = false;
  $('setup-retry').hidden = true;
  setupError('');
  try {
    const [state, inventory] = await Promise.all([send('/api/setup', 'GET'), send('/api/sensors', 'GET')]);
    if (inventory.sensors?.length) {
      showDashboard(inventory.sensors);
      return;
    }
    chosenKinds = state.choices.filter((kind) => sensorNames[kind]);
    renderSetup();
  } catch (error) {
    $('setup-loading').hidden = true;
    setupError(error.message);
    $('setup-retry').hidden = false;
  }
}

function showSetup(username) {
  signedInUsername = username;
  $('account-panel').hidden = true;
  $('dashboard').hidden = true;
  $('setup').hidden = false;
  document.querySelector('.shell').classList.add('setup-active');
  document.querySelector('.shell').classList.remove('dashboard-active');
  $('setup-username').textContent = username;
  loadSetup();
}

function showDashboard(sensors) {
  $('account-panel').hidden = true;
  $('setup').hidden = true;
  $('dashboard').hidden = false;
  document.querySelector('.shell').classList.remove('setup-active');
  document.querySelector('.shell').classList.add('dashboard-active');
  window.fizzDashboard.start(signedInUsername, sensors);
}

async function send(url, method, data) {
  const response = await fetch(url, {
    method,
    credentials: 'same-origin',
    headers: { 'content-type': 'application/json' },
    body: data ? JSON.stringify(data) : undefined,
  });
  const result = await response.json();
  if (!response.ok) throw new Error(result.error?.message || 'Something went wrong. Try again.');
  return result;
}

form.addEventListener('submit', async (event) => {
  event.preventDefault();
  if (!validUsername()) return;
  if (mode === 'create' && step === 1) {
    step = 2;
    render();
    $('code').focus();
    return;
  }
  if (!validCode()) return;
  const button = $('continue');
  button.disabled = true;
  showError('');
  try {
    const result = await send(mode === 'create' ? '/api/onboarding' : '/api/session', 'POST', {
      username: $('username').value.trim(), code: $('code').value,
    });
    form.reset();
    showSetup(result.username);
  } catch (error) {
    showError(error.message);
  } finally {
    button.disabled = false;
  }
});

document.querySelectorAll('.sensor-card').forEach((card) => {
  const info = window.fizzKinds.get(card.dataset.kind);
  const name = document.createElement('span');
  name.className = 'sensor-name';
  name.textContent = info.label;
  const detail = document.createElement('span');
  detail.className = 'sensor-detail';
  detail.textContent = info.detail;
  card.style.setProperty('--sensor-color', info.color);
  card.replaceChildren(window.fizzKinds.badge(card.dataset.kind, 'sensor-icon'), name, detail);
});
$('create-mode').addEventListener('click', () => setMode('create'));
$('sign-in-mode').addEventListener('click', () => setMode('sign-in'));
$('back').addEventListener('click', () => { step = 1; render(); $('username').focus(); });
$('edit-username').addEventListener('click', () => { step = 1; render(); $('username').focus(); });
document.querySelectorAll('.sensor-card').forEach((card) => card.addEventListener('click', async () => {
  const cards = document.querySelectorAll('.sensor-card');
  const removing = chosenKinds.includes(card.dataset.kind);
  cards.forEach((item) => { item.disabled = true; });
  setupError('');
  try {
    const state = await send('/api/setup', removing ? 'DELETE' : 'POST', { kind: card.dataset.kind });
    chosenKinds = state.choices.filter((kind) => sensorNames[kind]);
    renderSetup();
  } catch (error) {
    setupError(error.message);
  } finally {
    cards.forEach((item) => { item.disabled = false; });
  }
}));
$('setup-start').addEventListener('click', async () => {
  const button = $('setup-start');
  button.disabled = true;
  setupError('');
  try {
    const current = await send('/api/sensors', 'GET');
    for (const kind of chosenKinds) {
      if (!current.sensors.some((sensor) => sensor.kind === kind)) {
        await send('/api/sensors', 'POST', { kind, mode: 'simulated' });
      }
    }
    const inventory = await send('/api/sensors', 'GET');
    showDashboard(inventory.sensors);
  } catch (error) {
    setupError(error.message);
    button.disabled = chosenKinds.length === 0;
  }
});
$('setup-retry').addEventListener('click', loadSetup);
async function signOut() {
  try {
    await send('/api/session', 'DELETE');
    window.fizzDashboard.stop();
    $('setup').hidden = true;
    $('dashboard').hidden = true;
    $('account-panel').hidden = false;
    document.querySelector('.shell').classList.remove('setup-active');
    document.querySelector('.shell').classList.remove('dashboard-active');
    chosenKinds = [];
    setMode('sign-in');
  } catch { /* Keep the signed-in view if the server could not revoke the session. */ }
}
$('sign-out').addEventListener('click', signOut);
$('dashboard-sign-out').addEventListener('click', signOut);

fetch('/api/me', { credentials: 'same-origin', cache: 'no-store' })
  .then((response) => response.ok ? response.json() : null)
  .then((data) => { if (data?.username) showSetup(data.username); })
  .catch(() => {});
render();
