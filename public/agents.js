(() => {
  const $ = (id) => document.getElementById(id);
  const mcpUrl = `${location.origin}/api/mcp`;

  async function api(path, method = 'GET', body) {
    const response = await fetch(path, { method, credentials: 'same-origin', headers: { 'content-type': 'application/json' }, body: body ? JSON.stringify(body) : undefined });
    const data = await response.json().catch(() => ({}));
    if (!response.ok) throw new Error(data.error?.message || 'Please try again.');
    return data;
  }
  function node(tag, className, content) {
    const el = document.createElement(tag);
    if (className) el.className = className;
    if (content !== undefined) el.textContent = content;
    return el;
  }
  function when(value) {
    return value ? new Date(value).toLocaleString([], { dateStyle: 'medium', timeStyle: 'short' }) : 'never';
  }
  function copyButton(text) {
    const button = node('button', 'secondary', 'Copy');
    button.type = 'button';
    button.addEventListener('click', async () => {
      await navigator.clipboard.writeText(text).catch(() => {});
      button.textContent = 'Copied ✓';
      setTimeout(() => { button.textContent = 'Copy'; }, 1500);
    });
    return button;
  }

  // Ready-to-paste setup for the clients people use most; the token appears only here, once.
  function setupSnippets(token) {
    return [
      { label: 'Hermes', note: 'Add to the mcp_servers section of your Hermes config.yaml, then restart Hermes.',
        text: `mcp_servers:\n  fizz:\n    url: "${mcpUrl}"\n    headers:\n      Authorization: "Bearer ${token}"` },
      { label: 'Claude Code', note: 'Run this in a terminal on the machine where you use Claude Code.',
        text: `claude mcp add --transport http fizz ${mcpUrl} --header "Authorization: Bearer ${token}"` },
      { label: 'Other MCP clients', note: 'Any client that supports remote (Streamable HTTP) MCP servers with a custom header.',
        text: `URL:     ${mcpUrl}\nHeader:  Authorization: Bearer ${token}` },
    ];
  }

  function showSecret(token, name) {
    const box = $('agent-secret');
    const panel = node('div', 'secret-panel');
    panel.append(node('strong', '', `Agent token for ${name}`), node('p', '', 'Copy it now. It will not be shown again.'));
    panel.append(node('code', '', token), copyButton(token));
    const snippets = setupSnippets(token);
    const tabs = node('div', 'setup-tabs');
    tabs.setAttribute('role', 'tablist');
    const area = node('div', 'setup-snippet');
    const select = (index) => {
      [...tabs.children].forEach((tab, i) => tab.setAttribute('aria-selected', String(i === index)));
      const pre = node('pre');
      pre.append(node('code', '', snippets[index].text));
      area.replaceChildren(pre, node('p', '', snippets[index].note), copyButton(snippets[index].text));
    };
    snippets.forEach((snippet, index) => {
      const tab = node('button', 'secondary', snippet.label);
      tab.type = 'button';
      tab.setAttribute('role', 'tab');
      tab.addEventListener('click', () => select(index));
      tabs.append(tab);
    });
    panel.append(tabs, area);
    select(0);
    box.replaceChildren(panel);
    box.hidden = false;
  }

  function render(connections) {
    $('agent-count').textContent = `${connections.length} ${connections.length === 1 ? 'agent' : 'agents'}`;
    $('agent-empty').hidden = connections.length > 0;
    const list = $('agent-list');
    list.replaceChildren();
    for (const connection of connections) {
      const item = node('li', 'agent-item');
      const scope = node('span', `agent-scope${connection.can_create_alerts ? ' alerts' : ''}`, connection.can_create_alerts ? 'Can create alerts' : 'Read only');
      const meta = node('span', 'agent-meta', `Connected ${when(connection.created_at)} · Last used ${when(connection.last_used_at)}`);
      const revoke = node('button', 'sensor-remove', 'Revoke');
      revoke.type = 'button';
      revoke.addEventListener('click', async () => {
        if (!confirm(`Revoke ${connection.name}? It will lose access to Fizzlayer immediately.`)) return;
        revoke.disabled = true;
        try { await api('/api/agents', 'DELETE', { id: connection.id }); await refresh(); }
        catch (error) { $('page-error').textContent = error.message; revoke.disabled = false; }
      });
      item.append(node('strong', '', connection.name), scope, meta, revoke);
      list.append(item);
    }
  }

  async function refresh() {
    const result = await api('/api/agents');
    render(result.connections || []);
  }

  $('agent-form').addEventListener('submit', async (event) => {
    event.preventDefault();
    const button = $('agent-form').querySelector('button[type=submit]');
    const name = $('agent-name').value.trim();
    $('agent-message').textContent = '';
    if (!name) { $('agent-message').textContent = 'Name the agent first.'; $('agent-name').focus(); return; }
    button.disabled = true;
    try {
      const result = await api('/api/agents', 'POST', { name, can_create_alerts: $('agent-alerts').checked });
      showSecret(result.token, result.connection.name);
      $('agent-form').reset();
      await refresh();
    } catch (error) { $('agent-message').textContent = error.message; }
    finally { button.disabled = false; }
  });

  $('pair-form').addEventListener('submit', async (event) => {
    event.preventDefault();
    const button = $('pair-form').querySelector('button[type=submit]');
    const code = $('pair-code').value.trim();
    $('pair-message').textContent = '';
    if (!code) { $('pair-message').textContent = 'Enter the code shown by fizz pair.'; $('pair-code').focus(); return; }
    button.disabled = true;
    try {
      const result = await api('/api/devices', 'POST', { action: 'claim', code, name: $('pair-name').value.trim() });
      $('pair-message').textContent = `Claimed "${result.device_name}". Approve the request on the device to finish pairing.`;
      $('pair-form').reset();
    } catch (error) { $('pair-message').textContent = error.message; }
    finally { button.disabled = false; }
  });

  $('agents-sign-out').addEventListener('click', async () => {
    try { await api('/api/session', 'DELETE'); location.href = '/app.html'; }
    catch (error) { $('page-error').textContent = error.message; }
  });

  $('mcp-url').textContent = mcpUrl;
  $('install-cmd').textContent = `curl -fsSL ${location.origin}/install.sh | sh`;
  (async () => {
    try {
      const me = await api('/api/me');
      $('agents-username').textContent = me.username;
      await refresh();
    } catch { location.href = '/app.html'; }
  })();
})();
