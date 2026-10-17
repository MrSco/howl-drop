// The HowlDrop panel (panel.py serves it under Home Assistant's ingress).
// Every name, sender, link and text here comes from whoever sent the drop, so
// it only ever goes into the page as text (textContent), never as markup.
// Paths are relative: ingress serves this page under a per-session prefix.
'use strict';

const list = document.getElementById('drops');
const status = document.getElementById('status');
const viewer = document.getElementById('viewer');
const viewerBody = document.getElementById('viewer-body');
let drops = [];
let filter = 'all';
let lastJson = '';

const GLYPH = { image: '🖼️', video: '🎬', text: '📝', link: '🔗', file: '📄' };
const WORD = { image: 'Photo', video: 'Video', text: 'Text', link: 'Link', file: 'File' };

function el(tag, attrs = {}, text) {
  const node = document.createElement(tag);
  for (const [key, value] of Object.entries(attrs)) node.setAttribute(key, value);
  if (text !== undefined) node.textContent = text;
  return node;
}

function fileUrl(drop, extra = '') {
  return 'file?p=' + encodeURIComponent(drop.id) + '&v=' + drop.version + extra;
}

function size(bytes) {
  if (bytes < 1024) return bytes + ' B';
  const units = ['KB', 'MB', 'GB'];
  let n = bytes / 1024;
  let i = 0;
  while (n >= 1024 && i < units.length - 1) { n /= 1024; i++; }
  return (n < 10 ? n.toFixed(1) : Math.round(n)) + ' ' + units[i];
}

function when(seconds) {
  const date = new Date(seconds * 1000);
  const ago = (Date.now() - date.getTime()) / 1000;
  if (ago < 60) return 'just now';
  if (ago < 3600) return Math.floor(ago / 60) + ' min ago';
  const today = new Date();
  const time = date.toLocaleTimeString([], { hour: 'numeric', minute: '2-digit' });
  if (date.toDateString() === today.toDateString()) return 'today ' + time;
  const yesterday = new Date(today.getTime() - 86400000);
  if (date.toDateString() === yesterday.toDateString()) return 'yesterday ' + time;
  return date.toLocaleDateString([], { month: 'short', day: 'numeric' }) + ' ' + time;
}

function shown(drop) {
  if (filter === 'all') return true;
  if (filter === 'media') return drop.kind === 'image' || drop.kind === 'video';
  if (filter === 'text') return drop.kind === 'text' || drop.kind === 'link';
  return drop.kind === 'file';
}

function copyText(text, button) {
  const done = () => { button.textContent = 'Copied'; setTimeout(() => { button.textContent = 'Copy'; }, 1500); };
  if (navigator.clipboard && window.isSecureContext) {
    navigator.clipboard.writeText(text).then(done, () => fallbackCopy(text) && done());
  } else if (fallbackCopy(text)) {
    done();
  }
}

function fallbackCopy(text) {
  const area = el('textarea');
  area.value = text;
  area.style.position = 'fixed';
  area.style.opacity = '0';
  document.body.appendChild(area);
  area.select();
  let ok = false;
  try { ok = document.execCommand('copy'); } catch (e) { ok = false; }
  area.remove();
  return ok;
}

async function fullText(drop) {
  const reply = await fetch(fileUrl(drop));
  return reply.ok ? reply.text() : drop.text || '';
}

function preview(drop) {
  if (drop.kind === 'text') {
    return el('div', { class: 'preview text' }, drop.text || '');
  }
  if (drop.kind === 'link' && drop.url) {
    const box = el('a', { class: 'preview link', href: drop.url, target: '_blank', rel: 'noopener noreferrer' });
    box.append(el('span', { class: 'glyph' }, GLYPH.link), el('span', {}, drop.url));
    return box;
  }
  const box = el('button', { class: 'preview', type: 'button', 'aria-label': 'Open ' + drop.name });
  const placeholder = () => {
    const ext = drop.name.includes('.') ? drop.name.split('.').pop().toUpperCase() : '';
    const inner = el('span');
    inner.append(el('span', { class: 'glyph' }, GLYPH[drop.kind] || GLYPH.file), el('span', { class: 'ext' }, ext));
    box.textContent = '';
    box.append(inner);
  };
  if (drop.thumb) {
    const img = el('img', { src: 'thumb?p=' + encodeURIComponent(drop.id) + '&v=' + drop.version, alt: '', loading: 'lazy' });
    img.addEventListener('error', placeholder, { once: true });
    box.append(img);
  } else {
    placeholder();
  }
  box.addEventListener('click', () => open(drop));
  return box;
}

function open(drop) {
  if (!drop.inline || drop.kind === 'file') {
    window.location.href = fileUrl(drop, '&download=1');
    return;
  }
  viewerBody.textContent = '';
  document.getElementById('viewer-name').textContent = drop.name;
  document.getElementById('viewer-download').href = fileUrl(drop, '&download=1');
  if (drop.kind === 'video') {
    viewerBody.append(el('video', { src: fileUrl(drop), controls: '', autoplay: '', playsinline: '' }));
  } else {
    const img = el('img', { src: fileUrl(drop), alt: drop.name });
    img.addEventListener('error', () => {
      img.replaceWith(el('p', {}, 'This browser cannot show ' + drop.name + '. Use Download.'));
    }, { once: true });
    viewerBody.append(img);
  }
  viewer.hidden = false;
}

function closeViewer() {
  viewer.hidden = true;
  viewerBody.textContent = '';
}

async function remove(drop, button) {
  if (button.dataset.sure !== '1') {
    button.dataset.sure = '1';
    button.textContent = 'Delete?';
    setTimeout(() => { button.dataset.sure = ''; button.textContent = 'Delete'; }, 3000);
    return;
  }
  button.disabled = true;
  const reply = await fetch('api/delete', {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ id: drop.id }),
  });
  if (!reply.ok) {
    button.disabled = false;
    status.textContent = 'Could not delete ' + drop.name + '.';
    return;
  }
  await load(true);
}

function card(drop) {
  const item = el('li', { class: 'drop' });
  item.append(preview(drop));

  const meta = el('div', { class: 'meta' });
  meta.append(el('div', { class: 'name', title: drop.name }, drop.name));
  const from = drop.sender ? ' from ' + drop.sender : '';
  meta.append(el('div', { class: 'sub' }, WORD[drop.kind] + from));
  meta.append(el('div', { class: 'sub' }, when(drop.time) + ' · ' + size(drop.size)));
  item.append(meta);

  const actions = el('div', { class: 'actions' });
  if (drop.kind === 'text') {
    const copy = el('button', { type: 'button' }, 'Copy');
    copy.addEventListener('click', async () => copyText(await fullText(drop), copy));
    actions.append(copy);
  }
  if (drop.kind === 'link' && drop.url) {
    actions.append(el('a', { href: drop.url, target: '_blank', rel: 'noopener noreferrer' }, 'Open'));
    const copy = el('button', { type: 'button' }, 'Copy');
    copy.addEventListener('click', () => copyText(drop.url, copy));
    actions.append(copy);
  }
  actions.append(el('a', { href: fileUrl(drop, '&download=1') }, 'Download'));
  const del = el('button', { type: 'button', class: 'delete' }, 'Delete');
  del.addEventListener('click', () => remove(drop, del));
  actions.append(del);
  item.append(actions);
  return item;
}

function render() {
  list.textContent = '';
  const visible = drops.filter(shown);
  if (!drops.length) {
    status.textContent = 'Nothing yet. AirDrop something to this Home Assistant and it shows up here.';
  } else if (!visible.length) {
    status.textContent = 'Nothing of this kind.';
  } else {
    status.textContent = visible.length === drops.length ? '' : visible.length + ' of ' + drops.length;
  }
  for (const drop of visible) list.append(card(drop));
}

async function load(force = false) {
  try {
    const reply = await fetch('api/drops', { cache: 'no-store' });
    if (!reply.ok) throw new Error(reply.status);
    const json = await reply.text();
    if (!force && json === lastJson) return;
    lastJson = json;
    const data = JSON.parse(json);
    drops = data.drops;
    document.getElementById('folder').textContent = data.folder;
    render();
  } catch (e) {
    status.textContent = 'Could not reach HowlDrop. Is the add-on running?';
  }
}

for (const button of document.querySelectorAll('.filters button')) {
  button.addEventListener('click', () => {
    filter = button.dataset.filter;
    for (const other of document.querySelectorAll('.filters button')) other.classList.toggle('on', other === button);
    render();
  });
}
document.getElementById('viewer-close').addEventListener('click', closeViewer);
viewer.addEventListener('click', (event) => { if (event.target === viewer || event.target === viewerBody) closeViewer(); });
document.addEventListener('keydown', (event) => { if (event.key === 'Escape' && !viewer.hidden) closeViewer(); });

// --- Send to iPhone: uploads go to the add-on's send folder (outbox.py) --------

const sendButton = document.getElementById('send-button');
const sendInput = document.getElementById('send-input');
const sending = document.getElementById('sending');
const sendsList = document.getElementById('sends');
const uploads = [];  // this page's uploads still on their way to the add-on
let sendJobs = [];
let sendTimer = null;

const STATE_WORD = { waiting: 'Waiting', sending: 'Sending', sent: 'Sent', failed: 'Not sent', uploading: 'Uploading' };

function renderSends() {
  const rows = [...uploads, ...sendJobs];
  sending.hidden = sendButton.hidden;
  sendsList.textContent = '';
  for (const job of rows.slice(0, 8)) {
    const item = el('li');
    item.append(el('span', { class: 'chip ' + job.state }, STATE_WORD[job.state] || job.state),
      el('span', { class: 'send-name', title: job.name }, job.name),
      el('span', { class: 'send-detail' }, job.detail || ''));
    sendsList.append(item);
  }
}

async function loadSends() {
  clearTimeout(sendTimer);
  try {
    const reply = await fetch('api/send', { cache: 'no-store' });
    if (reply.ok) {
      const data = await reply.json();
      sendButton.hidden = !data.enabled;
      sendJobs = data.enabled ? data.jobs : [];
      renderSends();
    }
  } catch (e) { /* the drops list says when the add-on is away */ }
  // Quickly while something is on its way, slowly otherwise.
  const busy = uploads.length || sendJobs.some((j) => j.state === 'waiting' || j.state === 'sending');
  sendTimer = setTimeout(() => { if (!document.hidden) loadSends(); else sendTimer = setTimeout(loadSends, 2000); },
    busy ? 2000 : 15000);
}

function upload(file) {
  const job = { name: file.name, state: 'uploading', detail: 'to Home Assistant: 0%' };
  uploads.unshift(job);
  renderSends();
  return new Promise((resolve) => {
    const xhr = new XMLHttpRequest();
    xhr.open('POST', 'api/send?name=' + encodeURIComponent(file.name) +
      (chosen ? '&to=' + encodeURIComponent(chosen) : ''));
    xhr.setRequestHeader('Content-Type', 'application/octet-stream');
    xhr.upload.addEventListener('progress', (event) => {
      if (event.lengthComputable) {
        job.detail = 'to Home Assistant: ' + Math.round((event.loaded / event.total) * 100) + '%';
        renderSends();
      }
    });
    const done = (ok, why) => {
      uploads.splice(uploads.indexOf(job), 1);
      if (!ok) sendJobs.unshift({ name: file.name, state: 'failed', detail: why });
      renderSends();
      loadSends();
      resolve();
    };
    xhr.addEventListener('load', () => done(xhr.status === 200, 'upload failed: ' + (xhr.responseText || xhr.status)));
    xhr.addEventListener('error', () => done(false, 'upload failed: the add-on did not answer'));
    xhr.send(file);
  });
}

// --- which device: the latest scan's, or the add-on's default -------------------

const devicesBox = document.getElementById('devices');
const scanButton = document.getElementById('scan-button');
let nearby = { devices: [], scanning: false, time: 0 };
let chosen = '';
let scanUntil = 0;  // poll quickly until a scan this page asked for ends
try { chosen = localStorage.getItem('howldrop-send-to') || ''; } catch (e) { chosen = ''; }

function choose(name) {
  chosen = name;
  try { localStorage.setItem('howldrop-send-to', name); } catch (e) { /* remembered for this visit only */ }
  renderDevices();
}

function renderDevices() {
  const names = nearby.devices.map((d) => d.name);
  // A device chosen before stays on the list, even if this scan missed it.
  if (chosen && !names.includes(chosen)) names.push(chosen);
  devicesBox.textContent = '';
  const chip = (label, value, title) => {
    const button = el('button', { type: 'button', role: 'radio', 'aria-checked': String(chosen === value), title }, label);
    if (chosen === value) button.classList.add('on');
    button.addEventListener('click', () => choose(value));
    devicesBox.append(button);
  };
  chip('Default', '', 'the send_to option, or the only iPhone nearby');
  for (const name of names) chip(name, name, name);
  scanButton.disabled = nearby.scanning;
  scanButton.textContent = nearby.scanning ? 'Looking… (about 30 s)' : 'Look for devices';
  sendButton.textContent = chosen ? 'Send to ' + chosen : 'Send to iPhone';
}

async function loadNearby() {
  try {
    const reply = await fetch('api/nearby', { cache: 'no-store' });
    if (reply.ok) nearby = await reply.json();
  } catch (e) { /* tried again with the sends */ }
  renderDevices();
  if (Date.now() < scanUntil && (nearby.scanning || Date.now() < scanUntil - 55000)) {
    setTimeout(loadNearby, 2000);
  } else if (scanUntil) {
    scanUntil = 0;
    if (!nearby.devices.length) {
      sendsList.prepend(el('li', {}, 'Nothing answered: set the iPhone’s AirDrop to Everyone for 10 Minutes, unlock it, and look again.'));
    }
  }
}

scanButton.addEventListener('click', async () => {
  scanButton.disabled = true;
  scanButton.textContent = 'Looking… (about 30 s)';
  try {
    await fetch('api/scan', { method: 'POST', headers: { 'Content-Type': 'application/json' }, body: '{}' });
  } catch (e) { /* the poll shows nothing changed */ }
  scanUntil = Date.now() + 60000;
  setTimeout(loadNearby, 2000);
});

sendButton.addEventListener('click', () => sendInput.click());
sendInput.addEventListener('change', async () => {
  const files = [...sendInput.files];
  sendInput.value = '';
  for (const file of files) await upload(file);
});

load();
loadSends();
loadNearby();
setInterval(() => { if (!document.hidden) load(); }, 10000);
