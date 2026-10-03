const $ = (id) => document.getElementById(id);
const form = $('account-form');
let mode = 'create';
let step = 1;

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

function showWelcome(username) {
  $('auth').hidden = true;
  $('welcome').hidden = false;
  $('account-panel').setAttribute('aria-labelledby', 'panel-title');
  $('step-label').textContent = 'CONNECTED';
  $('welcome-name').textContent = username;
  $('panel-title').focus();
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
    showWelcome(result.username);
  } catch (error) {
    showError(error.message);
  } finally {
    button.disabled = false;
  }
});

$('create-mode').addEventListener('click', () => setMode('create'));
$('sign-in-mode').addEventListener('click', () => setMode('sign-in'));
$('back').addEventListener('click', () => { step = 1; render(); $('username').focus(); });
$('edit-username').addEventListener('click', () => { step = 1; render(); $('username').focus(); });
$('sign-out').addEventListener('click', async () => {
  try {
    await send('/api/session', 'DELETE');
    $('welcome').hidden = true;
    $('auth').hidden = false;
    $('account-panel').setAttribute('aria-labelledby', 'auth-title');
    setMode('sign-in');
  } catch { /* Keep the signed-in view if the server could not revoke the session. */ }
});

fetch('/api/me', { credentials: 'same-origin', cache: 'no-store' })
  .then((response) => response.ok ? response.json() : null)
  .then((data) => { if (data?.username) showWelcome(data.username); })
  .catch(() => {});
render();
