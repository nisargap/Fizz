(() => {
  const $ = (id) => document.getElementById(id);
  const passkey = window.fizzPasskey;
  const providerNames = { email: 'Email and password', google: 'Google' };
  let canAddPasskey = false;

  async function api(path, method = 'GET', body) {
    const response = await fetch(path, { method, credentials: 'same-origin', headers: { 'content-type': 'application/json' }, body: body ? JSON.stringify(body) : undefined });
    const data = await response.json().catch(() => ({}));
    if (!response.ok) throw Object.assign(new Error(data.error?.message || 'Please try again.'), { code: data.error?.code, status: response.status });
    return data;
  }
  function node(tag, className, content) {
    const el = document.createElement(tag);
    if (className) el.className = className;
    if (content !== undefined) el.textContent = content;
    return el;
  }
  const date = (value) => value ? new Date(value).toLocaleDateString([], { dateStyle: 'medium' }) : '—';
  const keyIcon = () => $('key-icon').content.firstElementChild.cloneNode(true);
  function keyBadge() {
    const badge = node('span', 'key-badge');
    badge.append(keyIcon());
    return badge;
  }
  function message(text) { $('passkey-message').textContent = text; }

  // A label for a new passkey, such as "Chrome on Mac", so the list tells them apart.
  function deviceName() {
    const ua = navigator.userAgent;
    const browser = /Edg\//.test(ua) ? 'Edge' : /Firefox\//.test(ua) ? 'Firefox' : /OPR\//.test(ua) ? 'Opera'
      : /Chrome\//.test(ua) ? 'Chrome' : /Safari\//.test(ua) ? 'Safari' : 'Browser';
    const device = /iPhone/.test(ua) ? 'iPhone' : /iPad/.test(ua) ? 'iPad' : /Android/.test(ua) ? 'Android'
      : /Macintosh|Mac OS X/.test(ua) ? 'Mac' : /Windows/.test(ua) ? 'Windows' : /CrOS/.test(ua) ? 'ChromeOS'
      : /Linux/.test(ua) ? 'Linux' : 'this device';
    return `${browser} on ${device}`;
  }

  function render(account) {
    $('settings-username').textContent = account.username;
    $('account-username').textContent = account.username;
    $('account-email').textContent = account.email || '—';
    const methods = (account.providers || []).map((p) => providerNames[p] || p.charAt(0).toUpperCase() + p.slice(1));
    if (account.passkeys.length) methods.push('Passkey');
    $('account-methods').textContent = methods.join(', ') || '—';
    $('account-since').textContent = date(account.created_at);
    canAddPasskey = account.can_add_passkey;

    const list = $('passkey-list');
    list.replaceChildren();
    for (const key of account.passkeys) {
      const item = node('li', 'passkey-item');
      const text = node('div', 'passkey-text');
      const used = key.last_used_at ? `Last used ${date(key.last_used_at)}` : 'Not used yet';
      text.append(node('strong', '', key.name || 'Passkey'), node('span', 'passkey-meta', `Added ${date(key.created_at)} · ${used}`));
      const remove = node('button', 'sensor-remove', 'Remove');
      remove.type = 'button';
      remove.setAttribute('aria-label', `Remove ${key.name || 'passkey'}`);
      remove.addEventListener('click', async () => {
        if (!confirm(`Remove ${key.name || 'this passkey'}? You will no longer be able to sign in with it. You can also delete it from your device's password manager.`)) return;
        remove.disabled = true;
        try {
          await api('/api/account', 'DELETE', { passkey_id: key.id });
          message('Passkey removed.');
          await refresh();
        } catch (error) {
          message(error.message);
          remove.disabled = false;
        }
      });
      item.append(keyBadge(), text, remove);
      list.append(item);
    }
    $('passkey-empty').hidden = account.passkeys.length > 0;
  }

  async function refresh() { render(await api('/api/account')); }

  // Supabase only registers a passkey for someone who signed in within the last hour.
  function needsRecentSignIn() {
    message('For security, adding a passkey needs a sign-in from the last hour. Sign in again, then press + right away.');
    $('sign-in-again').hidden = false;
  }

  $('add-passkey').addEventListener('click', async () => {
    message('');
    if (!passkey.supported) { message('This browser does not support passkeys. Try a current version of Chrome, Safari, Edge, or Firefox.'); return; }
    if (!canAddPasskey) { needsRecentSignIn(); return; }
    const button = $('add-passkey');
    button.disabled = true;
    try {
      const start = await api('/api/auth', 'POST', { action: 'passkey_add_options' });
      const credential = await passkey.credential(start.options, true);
      await api('/api/auth', 'POST', { action: 'passkey_add', challenge_id: start.challenge_id, credential, name: deviceName() });
      message('Passkey added. Next time, choose "Sign in with a passkey".');
      await refresh();
    } catch (error) {
      if (passkey.cancelled(error)) return;
      if (error.code === 'sign_in_again') { needsRecentSignIn(); return; }
      message(error instanceof DOMException ? 'That passkey could not be saved. Try again.' : error.message);
    } finally {
      button.disabled = false;
    }
  });

  async function signOut(next) {
    try { await api('/api/session', 'DELETE'); location.href = next; }
    catch (error) { $('page-error').textContent = error.message; }
  }
  $('settings-sign-out').addEventListener('click', () => signOut('/app.html'));
  $('sign-in-again').addEventListener('click', () => signOut('/app.html#sign-in'));

  for (const slot of document.querySelectorAll('[data-key-icon]')) slot.append(keyIcon());
  refresh().catch((error) => {
    if (error.status === 401) location.href = '/app.html#sign-in';
    else $('page-error').textContent = error.message;
  });
})();
