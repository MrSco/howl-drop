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

load();
setInterval(() => { if (!document.hidden) load(); }, 10000);
