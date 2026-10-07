const $ = (id) => document.getElementById(id);
const form = $('account-form');
const sensorNames = {
  water: 'Water', gas: 'Gas', radio: 'Radio', temperature: 'Temperature',
  pressure: 'Pressure', humidity: 'Humidity', sound: 'Sound', phone: 'Phone', custom: 'Custom',
};
let chosenKinds = [];
let signedInUsername = '';

// Sign-in goes through Supabase Auth via /api/auth: email and password, Google, or a passkey.
// New accounts need an invite. Passkeys can only be added once signed in.
const passkeysSupported = Boolean(window.PublicKeyCredential?.parseRequestOptionsFromJSON && navigator.credentials);
const modes = {
  create: {
    label: 'NEW ACCOUNT', title: 'Create your account', intro: 'Enter your invite code, your email, and a password.',
    submit: 'Create account', fields: ['invite', 'email', 'password'], google: 'Sign up with Google',
  },
  'sign-in': {
    label: 'SIGN IN', title: 'Welcome back', intro: 'Sign in with your email and password, Google, or a passkey.',
    submit: 'Sign in', fields: ['email', 'password'], google: 'Sign in with Google', passkey: true,
  },
  forgot: {
    label: 'RESET', title: 'Reset your password', intro: 'Enter your email and we will send you a link to choose a new password.',
    submit: 'Send reset link', fields: ['email'],
  },
  reset: {
    label: 'NEW PASSWORD', title: 'Choose a new password', intro: 'Enter a new password for your Fizzlayer account.',
    submit: 'Save password', fields: ['password'],
  },
  'check-email': { label: 'CHECK EMAIL', title: 'Check your email', intro: '', fields: [] },
};
let mode = 'create';
let resetToken = ''; // From a password reset link; used once to set the new password.
let pending = null; // What "Resend email" sends: { action, email }.

function showError(message) {
  $('form-error').textContent = message;
  $('form-error').hidden = !message;
}

function render(intro) {
  const current = modes[mode];
  const choosing = mode === 'create' || mode === 'sign-in';
  $('mode-switch').hidden = !choosing;
  for (const [id, on] of [['create-mode', mode === 'create'], ['sign-in-mode', mode === 'sign-in']]) {
    $(id).classList.toggle('selected', on);
    $(id).setAttribute('aria-pressed', String(on));
  }
  for (const field of ['invite', 'email', 'password']) $(`${field}-wrap`).hidden = !current.fields.includes(field);
  // The form stays visible in every mode so its error message can show; only its parts hide.
  $('form-actions').hidden = !current.submit;
  $('email-sent').hidden = mode !== 'check-email';
  $('oauth').hidden = !choosing;
  $('google-label').textContent = current.google || 'Continue with Google';
  $('passkey').hidden = !(current.passkey && passkeysSupported);
  $('forgot').hidden = mode !== 'sign-in' && mode !== 'forgot';
  $('forgot').textContent = mode === 'forgot' ? 'Back to sign in' : 'Forgot password?';
  $('step-label').textContent = current.label;
  $('auth-title').textContent = current.title;
  $('auth-intro').textContent = intro ?? current.intro;
  $('continue').textContent = current.submit || '';
  $('password').autocomplete = mode === 'sign-in' ? 'current-password' : 'new-password';
  $('email').autocomplete = mode === 'sign-in' ? 'username' : 'email';
  $('password-help').hidden = mode === 'sign-in';
  showError('');
}

function setMode(next, intro) {
  const email = $('email').value;
  mode = next;
  form.reset();
  $('email').value = email; // Keep a typed address when moving between sign-in and reset.
  render(intro);
  const first = modes[next].fields.find((field) => !(field === 'email' && email));
  if (first) $(first).focus();
}

// Invite codes look like FIZZ-ABCD-EFGH-JKMN; the server does the real check.
function validInvite() {
  const symbols = $('invite').value.replace(/[^A-Za-z0-9]/g, '').length;
  if (symbols < 12 || symbols > 16) {
    showError('Enter the invite code from your Fizzlayer invitation.');
    $('invite').focus();
    return false;
  }
  return true;
}

function validEmail() {
  if (!/^[^\s@]+@[^\s@]+\.[^\s@]+$/.test($('email').value.trim())) {
    showError('Enter a valid email address.');
    $('email').focus();
    return false;
  }
  return true;
}

function validPassword() {
  const length = $('password').value.length;
  if (length < 8 || length > 72) {
    showError(mode === 'sign-in' ? 'Enter your password.' : 'Use a password of 8 to 72 characters.');
    $('password').focus();
    return false;
  }
  return true;
}

// Passkey options and responses use WebAuthn's JSON form, so they pass straight through.
async function passkeyCredential(options, creating) {
  const publicKey = options.publicKey || options;
  const credential = creating
    ? await navigator.credentials.create({ publicKey: PublicKeyCredential.parseCreationOptionsFromJSON(publicKey) })
    : await navigator.credentials.get({ publicKey: PublicKeyCredential.parseRequestOptionsFromJSON(publicKey) });
  return credential.toJSON();
}

// The browser throws NotAllowedError when the person closes the passkey prompt.
const cancelled = (error) => error?.name === 'NotAllowedError' || error?.name === 'AbortError';

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
  if (!response.ok) throw Object.assign(new Error(result.error?.message || 'Something went wrong. Try again.'), { code: result.error?.code });
  return result;
}

form.addEventListener('submit', async (event) => {
  event.preventDefault();
  const { fields } = modes[mode];
  if (fields.includes('invite') && !validInvite()) return;
  if (fields.includes('email') && !validEmail()) return;
  if (fields.includes('password') && !validPassword()) return;
  const email = $('email').value.trim();
  const password = $('password').value;
  const body = {
    create: { action: 'sign_up', invite: $('invite').value.trim(), email, password },
    'sign-in': { action: 'sign_in', email, password },
    forgot: { action: 'recover', email },
    reset: { action: 'link', access_token: resetToken, password },
  }[mode];
  const button = $('continue');
  button.disabled = true;
  showError('');
  try {
    const result = await send('/api/auth', 'POST', body);
    if (result.username) {
      resetToken = '';
      form.reset();
      showSetup(result.username);
    } else if (mode === 'forgot') {
      pending = { action: 'recover', email };
      setMode('check-email', `If ${email} has a Fizzlayer account, we sent it a link to choose a new password.`);
    } else {
      pending = { action: 'resend', email };
      setMode('check-email', `We sent a confirmation link to ${email}. Open it on any device to finish creating your account.`);
    }
  } catch (error) {
    if (error.code === 'email_not_confirmed') {
      pending = { action: 'resend', email };
      setMode('check-email', `Confirm your email first. Open the link we sent to ${email}, or send it again.`);
      return;
    }
    if (error.code === 'link_expired' && mode === 'reset') {
      setMode('forgot');
      showError(error.message);
      return;
    }
    showError(error.message);
    const field = { invalid_invite: 'invite', invalid_email: 'email', weak_password: 'password', same_password: 'password' }[error.code];
    if (field && modes[mode].fields.includes(field)) $(field).focus();
  } finally {
    button.disabled = false;
  }
});

$('resend').addEventListener('click', async () => {
  if (!pending) return;
  const button = $('resend');
  button.disabled = true;
  showError('');
  try {
    await send('/api/auth', 'POST', pending);
    $('auth-intro').textContent = `Sent again to ${pending.email}. Check your inbox and spam folder.`;
  } catch (error) {
    showError(error.message);
  } finally {
    button.disabled = false;
  }
});

$('passkey').addEventListener('click', async () => {
  const button = $('passkey');
  button.disabled = true;
  showError('');
  try {
    const start = await send('/api/auth', 'POST', { action: 'passkey_options' });
    const credential = await passkeyCredential(start.options, false);
    const result = await send('/api/auth', 'POST', { action: 'passkey_sign_in', challenge_id: start.challenge_id, credential });
    showSetup(result.username);
  } catch (error) {
    if (!cancelled(error)) showError(error instanceof DOMException ? 'That passkey didn’t work. Try again.' : error.message);
  } finally {
    button.disabled = false;
  }
});

async function addPasskey(button) {
  const label = button.textContent;
  button.disabled = true;
  try {
    const start = await send('/api/auth', 'POST', { action: 'passkey_add_options' });
    const credential = await passkeyCredential(start.options, true);
    await send('/api/auth', 'POST', { action: 'passkey_add', challenge_id: start.challenge_id, credential });
    button.textContent = 'Passkey added';
    setTimeout(() => { button.textContent = label; button.disabled = false; }, 4000);
    return;
  } catch (error) {
    if (!cancelled(error)) alert(error instanceof DOMException ? 'That passkey could not be saved. Try again.' : error.message);
  }
  button.disabled = false;
}
for (const id of ['add-passkey', 'dashboard-passkey']) {
  $(id).hidden = !passkeysSupported;
  $(id).addEventListener('click', () => addPasskey($(id)));
}

$('create-mode').addEventListener('click', () => setMode('create'));
$('sign-in-mode').addEventListener('click', () => setMode('sign-in'));
$('forgot').addEventListener('click', () => setMode(mode === 'forgot' ? 'sign-in' : 'forgot'));
$('back-to-sign-in').addEventListener('click', () => setMode('sign-in'));
// Google sign-in leaves the page; new accounts still need an invite, which rides along.
$('google').addEventListener('click', () => {
  if (mode === 'create' && !validInvite()) return;
  const invite = mode === 'create' ? `&invite=${encodeURIComponent($('invite').value.trim())}` : '';
  location.assign(`/api/auth?provider=google${invite}`);
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

// Arriving here can carry state in the URL fragment:
// - #sign-in from the landing page's Sign in link.
// - #auth_error=<code> when Google sign-in fails in /api/auth.
// - #access_token=...&type=signup|recovery from a Supabase email link, or #error=... if it expired.
const hash = new URLSearchParams(location.hash.slice(1));
const authErrors = {
  no_account: ['create', 'No Fizzlayer account uses that Google account yet. Enter your invite code to sign up with Google.', 'invite'],
  invalid_invite: ['create', 'That invite code is not valid or has already been used.', 'invite'],
  rate_limited: [null, 'Too many attempts. Try again in an hour.'],
  cancelled: [null, 'Google sign-in was cancelled.'],
  expired: [null, 'Google sign-in took too long or was interrupted. Try again.'],
  failed: [null, 'Google sign-in is temporarily unavailable. Try again.'],
};
const linkToken = hash.get('access_token');
const authError = hash.get('auth_error');
if (location.hash === '#sign-in') mode = 'sign-in';
// Tokens must not linger in the address bar or history.
if (location.hash) history.replaceState(null, '', location.pathname);

if (linkToken && hash.get('type') === 'recovery') {
  resetToken = linkToken;
  mode = 'reset';
  render();
  $('password').focus();
} else if (linkToken) {
  mode = 'sign-in';
  render('Confirming your email…');
  send('/api/auth', 'POST', { action: 'link', access_token: linkToken })
    .then((result) => showSetup(result.username))
    .catch((error) => {
      const fix = authErrors[error.code];
      if (fix?.[0]) mode = fix[0];
      render();
      showError(error.message);
    });
} else if (hash.get('error') || hash.get('error_code')) {
  mode = 'sign-in';
  render();
  showError('That link has expired or was already used. Request a new one.');
} else {
  if (authError) mode = (authErrors[authError] || authErrors.failed)[0] || mode;
  render();
  if (authError) {
    const [, message, field] = authErrors[authError] || authErrors.failed;
    showError(message);
    if (field) $(field).focus();
  }
  fetch('/api/me', { credentials: 'same-origin', cache: 'no-store' })
    .then((response) => response.ok ? response.json() : null)
    .then((data) => { if (data?.username) showSetup(data.username); })
    .catch(() => {});
}
