const { invoke, listen, WebviewWindow, appWindow } = window.__TAURI__;

const $ = (sel) => document.querySelector(sel);
const $$ = (sel) => document.querySelectorAll(sel);

let logAutoScroll = true;

// ---------- Utilities ----------
function log(source, level, message, data = null) {
  const output = $('#logOutput');
  const time = new Date().toLocaleTimeString();
  const line = document.createElement('div');
  line.className = `log-line ${level}`;
  line.innerHTML = `
    <span class="log-timestamp">${time}</span>
    <span class="log-source">[${source}]</span>
    <span>${escapeHtml(message)}</span>
  `;
  output.appendChild(line);
  if (logAutoScroll) output.scrollTop = output.scrollHeight;
}

function escapeHtml(str) {
  return String(str).replace(/[&<>"']/g, c => ({'&':'&','<':'<','>':'>','"':'"',"'":'''}[c]));
}

function setStatus(connected) {
  const dot = $('#statusDot');
  const text = $('#statusText');
  const indicator = $('#statusIndicator');
  if (connected) {
    dot.style.background = 'var(--success)';
    dot.style.boxShadow = '0 0 8px var(--success)';
    text.textContent = 'Connected';
    indicator.classList.add('connected');
  } else {
    dot.style.background = 'var(--fg-subtle)';
    dot.style.boxShadow = 'none';
    text.textContent = 'Disconnected';
    indicator.classList.remove('connected');
  }
}

function showTab(tabName) {
  $$('.panel').forEach(p => p.classList.remove('active'));
  $$('.nav-item').forEach(n => n.classList.remove('active'));
  $(`#panel-${tabName}`).classList.add('active');
  $(`[data-tab="${tabName}"]`).classList.add('active');
}

// ---------- Titlebar Controls ----------
$('#minimizeBtn').onclick = () => appWindow.minimize();
$('#maximizeBtn').onclick = () => appWindow.toggleMaximize();
$('#closeBtn').onclick = () => appWindow.close();

// Drag titlebar
const titlebar = $('#titlebar');
titlebar.addEventListener('mousedown', (e) => {
  if (e.target.closest('.icon-btn')) return;
  appWindow.startDragging();
});

// ---------- Navigation ----------
$$('.nav-item').forEach(btn => {
  btn.onclick = () => showTab(btn.dataset.tab);
});

// ---------- Health Check ----------
async function healthCheck() {
  try {
    const res = await invoke('health_check');
    setStatus(res.llm_healthy || res.scheduler_running);
    log('System', 'info', `Health: LLM=${res.llm_healthy}, Scheduler=${res.scheduler_running}`);
  } catch (e) {
    setStatus(false);
    log('System', 'error', 'Health check failed', e);
  }
}
$('#healthCheckBtn').onclick = healthCheck;

// ---------- DAG Scheduler ----------
let schedulerInitialized = false;

$('#initSchedulerBtn').onclick = async () => {
  const concurrency = parseInt($('#dagConcurrency').value) || 4;
  try {
    await invoke('init_scheduler', { max_concurrent: concurrency });
    schedulerInitialized = true;
    $('#runSchedulerBtn').disabled = false;
    log('DAG', 'success', `Scheduler initialized (concurrency: ${concurrency})`);
    refreshTasks();
  } catch (e) { log('DAG', 'error', 'Init failed', e); }
};

$('#addTaskBtn').onclick = async () => {
  const name = $('#taskName').value.trim();
  const payloadStr = $('#taskPayload').value.trim();
  const depsStr = $('#taskDeps').value.trim();
  if (!name) return log('DAG', 'warn', 'Task name required');
  let payload = {};
  try { payload = JSON.parse(payloadStr || '{}'); } catch { return log('DAG', 'error', 'Invalid JSON payload'); }
  const dependencies = depsStr ? depsStr.split(',').map(s => s.trim()).filter(Boolean) : [];
  try {
    const { task_id } = await invoke('add_task', { request: { name, payload, dependencies } });
    log('DAG', 'success', `Task added: ${task_id}`);
    refreshTasks();
  } catch (e) { log('DAG', 'error', 'Add task failed', e); }
};

$('#runSchedulerBtn').onclick = async () => {
  try {
    log('DAG', 'info', 'Running scheduler...');
    await invoke('run_scheduler');
    log('DAG', 'success', 'Scheduler completed');
    refreshTasks();
  } catch (e) { log('DAG', 'error', 'Run failed', e); }
};

$('#refreshTasksBtn').onclick = refreshTasks;

async function refreshTasks() {
  try {
    const tasks = await invoke('list_tasks');
    const tbody = $('#taskTable tbody');
    tbody.innerHTML = tasks.map(t => `
      <tr>
        <td><code>${t.id.slice(0,8)}…</code></td>
        <td>${escapeHtml(t.name)}</td>
        <td><span class="status-badge ${t.status.toLowerCase()}">${t.status}</span></td>
        <td>${t.dependencies.map(d => `<code>${d.slice(0,8)}…</code>`).join(', ') || '—'}</td>
        <td>${t.error ? `<span style="color:var(--error)">${escapeHtml(t.error)}</span>` : '—'}</td>
      </tr>
    `).join('') || '<tr><td colspan="5" style="text-align:center;color:var(--fg-subtle)">No tasks</td></tr>';
    const status = await invoke('get_scheduler_status');
    log('DAG', 'info', `Status: ${status.tasks_completed}/${status.tasks_scheduled} done, ${status.running_count} running`);
  } catch (e) { log('DAG', 'error', 'Refresh failed', e); }
}

// ---------- LLM ----------
let llmConfigured = false;

$('#configureLLMBtn').onclick = async () => {
  const provider = $('#llmProvider').value;
  const model = $('#llmModel').value.trim();
  const apiKey = $('#llmApiKey').value.trim();
  if (!model) return log('LLM', 'warn', 'Model required');
  try {
    await invoke('configure_llm', {
      request: { provider, model, api_key: apiKey || undefined }
    });
    llmConfigured = true;
    log('LLM', 'success', `Configured: ${provider}/${model}`);
  } catch (e) { log('LLM', 'error', 'Configure failed', e); }
};

function appendMessage(role, content) {
  const list = $('#messageList');
  const div = document.createElement('div');
  div.className = `message ${role}`;
  div.innerHTML = `
    <div class="avatar">${role[0].toUpperCase()}</div>
    <div class="content"><pre>${escapeHtml(content)}</pre></div>
  `;
  list.appendChild(div);
  list.scrollTop = list.scrollHeight;
  return div.querySelector('.content pre');
}

$('#sendMessageBtn').onclick = async () => {
  const input = $('#userInput').value.trim();
  const stream = $('#streamToggle').checked;
  if (!input) return;
  if (!llmConfigured) return log('LLM', 'warn', 'Configure LLM first');

  $('#userInput').value = '';
  appendMessage('user', input);

  const messages = [{ role: 'user', content: { text: input }, name: null, tool_calls: null, tool_call_id: null }];

  if (stream) {
    const pre = appendMessage('assistant', '');
    let fullContent = '';
    try {
      await invoke('llm_stream', { request: { messages, options: { stream: true } } });
      // Events handled via listener below
    } catch (e) { log('LLM', 'error', 'Stream invoke failed', e); }
  } else {
    try {
      const res = await invoke('llm_complete', { request: { messages, options: { stream: false } } });
      appendMessage('assistant', res.content);
      log('LLM', 'success', 'Completion received');
    } catch (e) { log('LLM', 'error', 'Complete failed', e); }
  }
};

// LLM Stream Events
listen('llm-stream-chunk', (event) => {
  const { chunk, is_final } = event.payload;
  const list = $('#messageList');
  const lastMsg = list.lastElementChild;
  if (!lastMsg || !lastMsg.classList.contains('assistant')) return;
  const pre = lastMsg.querySelector('pre');
  if (chunk.choices?.[0]?.delta?.content) {
    pre.textContent += chunk.choices[0].delta.content;
    list.scrollTop = list.scrollHeight;
  }
  if (is_final) {
    log('LLM', 'success', 'Stream completed');
  }
});

listen('llm-stream-error', (event) => {
  log('LLM', 'error', 'Stream error', event.payload);
});

// ---------- Tools: Shell ----------
$('#runShellBtn').onclick = async () => {
  const command = $('#shellCmd').value.trim();
  const args = $('#shellArgs').value.split(',').map(s => s.trim()).filter(Boolean);
  const cwd = $('#shellCwd').value.trim() || undefined;
  const timeout = parseInt($('#shellTimeout').value) || 30;
  if (!command) return log('Tools', 'warn', 'Command required');
  try {
    log('Shell', 'info', `$ ${command} ${args.join(' ')}`);
    const res = await invoke('shell_execute', {
      request: { command, args, working_dir: cwd, environment: {}, timeout_secs: timeout }
    });
    $('#shellOutput').textContent = res.stdout || res.stderr || '(no output)';
    log('Shell', res.success ? 'success' : 'error', `Exit: ${res.exit_code} (${res.duration_ms}ms)`);
  } catch (e) { log('Shell', 'error', 'Execute failed', e); }
};

// ---------- Tools: File ----------
$('#runFileBtn').onclick = async () => {
  const op = $('#fileOp').value;
  const path = $('#filePath').value.trim();
  const destination = $('#fileDest').value.trim() || undefined;
  const content = $('#fileContent').value;
  if (!path) return log('Tools', 'warn', 'Path required');
  try {
    log('File', 'info', `${op.toUpperCase()} ${path}${destination ? ` -> ${destination}` : ''}`);
    const res = await invoke('file_operation', {
      request: { op, path, destination, content: content || undefined, encoding: 'utf-8', recursive: true, create_parents: true }
    });
    $('#fileOutput').textContent = res.stdout || res.stderr || '(no output)';
    log('File', res.success ? 'success' : 'error', `Done (${res.duration_ms}ms)`);
  } catch (e) { log('File', 'error', 'Execute failed', e); }
};

// ---------- Logs ----------
$('#clearLogsBtn').onclick = () => { $('#logOutput').innerHTML = ''; };
$('#autoScrollLogs').onchange = (e) => { logAutoScroll = e.target.checked; };

// ---------- Sandbox Config ----------
async function loadSandboxConfig() {
  // Could load saved config here
}

// ---------- Init ----------
async function init() {
  log('System', 'info', 'Chimera UI loaded');
  await healthCheck();
  setInterval(healthCheck, 30000);
}

document.addEventListener('DOMContentLoaded', init);