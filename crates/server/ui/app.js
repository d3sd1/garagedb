// GarageDB UI — vanilla, sin CDN, offline-first.
'use strict';

const $ = (s) => document.querySelector(s);
const ACTOR = localStorage.getItem('gdb_actor') || 'taller';

function toast(msg, ms = 2500) {
  const t = $('#toast');
  t.textContent = msg;
  t.classList.add('show');
  setTimeout(() => t.classList.remove('show'), ms);
}

async function api(path, opts) {
  const res = await fetch(path, opts);
  const data = await res.json().catch(() => ({}));
  if (!res.ok) throw Object.assign(new Error(data.error || res.statusText), { data, status: res.status });
  return data;
}

async function postEvent(body) {
  return api('/api/event', {
    method: 'POST',
    headers: { 'content-type': 'application/json' },
    body: JSON.stringify({ actor: ACTOR, body }),
  });
}

// ---------- tabs ----------
document.querySelectorAll('nav button').forEach((b) => {
  b.addEventListener('click', () => {
    document.querySelectorAll('nav button').forEach((x) => x.classList.remove('active'));
    b.classList.add('active');
    document.querySelectorAll('main > section').forEach((s) => s.classList.add('hide'));
    $('#tab-' + b.dataset.tab).classList.remove('hide');
    if (b.dataset.tab === 'stock') loadStock();
    if (b.dataset.tab === 'misiones') loadMissions();
    if (b.dataset.tab === 'sync') loadSummary();
  });
});

// ---------- summary / header ----------
async function loadSummary() {
  try {
    const s = await api('/api/summary');
    $('#syncdot').className = 'dot ' + (s.n_rejected > 0 || s.malformed_lines > 0 ? 'warn' : 'ok');
    $('#syncinfo').textContent =
      `réplica ${s.replica} · ${s.n_events} eventos · ${s.n_proposals} propuestas · ${s.last_fold}`;
    $('#syncdetail').innerHTML =
      `<p><strong>Réplica:</strong> ${s.replica}</p>` +
      `<p><strong>Eventos:</strong> ${s.n_events} · <strong>Rechazados:</strong> ${s.n_rejected} · ` +
      `<strong>Líneas malformadas:</strong> ${s.malformed_lines}</p>` +
      `<p><strong>Propuestas pendientes:</strong> ${s.n_proposals} · <strong>Anomalías:</strong> ${s.n_anomalies}</p>` +
      `<p class="muted">Último fold: ${s.last_fold}</p>`;
  } catch (e) {
    $('#syncdot').className = 'dot';
    $('#syncinfo').textContent = 'sin conexión con el servidor';
  }
}

async function doSync() {
  try {
    const s = await api('/api/sync', { method: 'POST' });
    toast(`Sincronizado: ${s.n_events} eventos`);
    loadSummary();
    loadStock();
  } catch (e) {
    toast('Error: ' + e.message);
  }
}
window.doSync = doSync;

// ---------- stock ----------
function qtyLabel(q) {
  if (q.kind === 'exact') return q.n;
  if (q.kind === 'estimated') return `~${q.n} (${q.lo}–${q.hi})`;
  const lv = { full: 'LLENO', half: 'MEDIO', low: 'BAJO', empty: 'VACÍO' };
  return lv[q.level] || q.level;
}

async function loadStock() {
  const q = $('#q').value.trim();
  const rows = await api('/api/stock?q=' + encodeURIComponent(q));
  const html = rows
    .map(
      (r, i) => `
    <div class="card">
      <div style="display:flex;justify-content:space-between;gap:8px;align-items:baseline">
        <div><strong>${r.sku}</strong> <span class="muted">${r.name || ''}</span><br>
          <span class="muted">${r.loc}</span>
          ${r.stale ? '<span class="badge stale">REVISAR</span>' : ''}</div>
        <div class="qty" style="font-size:24px">${qtyLabel(r.qty)} <span class="muted" style="font-size:14px">${r.unit || ''}</span></div>
      </div>
      <div>
        <button class="act sec" onclick="move('${r.sku}','${r.loc}',-1)">−1</button>
        <button class="act sec" onclick="move('${r.sku}','${r.loc}',1)">+1</button>
        <button class="act sec" onclick="moveN('${r.sku}','${r.loc}')">±n</button>
      </div>
    </div>`
    )
    .join('');
  $('#stocklist').innerHTML = html || '<p class="muted">Sin resultados. Da de alta material en Ingesta.</p>';
}

async function move(sku, loc, delta, reason) {
  try {
    await postEvent({ op: 'move', sku, loc, delta, reason: reason || 'ajuste rápido', mission: null });
    loadStock();
    loadSummary();
  } catch (e) {
    toast('Error: ' + e.message);
  }
}
window.move = move;

function moveN(sku, loc) {
  const v = prompt(`Δ para ${sku} en ${loc} (ej. -4 consumo, +100 pedido):`);
  if (!v) return;
  const delta = parseInt(v, 10);
  if (!delta) return toast('Delta inválido');
  const reason = prompt('Motivo:') || 'ajuste manual';
  move(sku, loc, delta, reason);
}
window.moveN = moveN;

let qTimer;
$('#q').addEventListener('input', () => {
  clearTimeout(qTimer);
  qTimer = setTimeout(loadStock, 250);
});

// ---------- ingesta ----------
$('#in-kind').addEventListener('change', () => {
  const presence = $('#in-kind').value === 'presence';
  $('#in-level').classList.toggle('hide', !presence);
  $('#in-n').classList.toggle('hide', presence);
});

async function submitIngest() {
  const sku = $('#in-sku').value.trim();
  const loc = $('#in-loc').value.trim();
  if (!sku || !loc) return toast('SKU y ubicación son obligatorios');
  const kind = $('#in-kind').value;
  let body;
  if (kind === 'presence') {
    body = {
      op: 'count', sku, loc,
      qty: { kind: 'presence', level: $('#in-level').value },
      source: 'human', status: 'confirmed',
    };
  } else {
    const n = parseInt($('#in-n').value, 10);
    if (isNaN(n) || n < 0) return toast('Cantidad inválida');
    body = kind === 'ingest_exact'
      ? { op: 'ingest', sku, name: $('#in-name').value.trim() || sku, category: 'general', unit: 'ud', loc, qty: { kind: 'exact', n } }
      : { op: 'count', sku, loc, qty: { kind: 'exact', n }, source: 'human', status: 'confirmed' };
  }
  try {
    await postEvent(body);
    toast('Guardado ✓');
    $('#in-n').value = '';
  } catch (e) {
    toast('Error: ' + e.message);
  }
}
window.submitIngest = submitIngest;

// ---------- misiones ----------
async function loadMissions() {
  const ms = await api('/api/missions');
  $('#missiondetail').innerHTML = '';
  $('#missionlist').innerHTML =
    ms
      .map(
        (m) => `
    <div class="card">
      <strong>${m.id}</strong> — ${m.circuit} (${m.date})<br>
      <span class="muted">kit ${m.kit} · carro ${m.vehicle} · estado <strong>${m.state}</strong></span><br>
      <button class="act" onclick="checkMission('${m.id}')">Check</button>
      <button class="act sec" onclick="readyMission('${m.id}')">Marcar LISTO</button>
    </div>`
      )
      .join('') || '<p class="muted">Sin misiones. Créalas desde la CLI: <code>garagedb mission create…</code></p>';
}

async function checkMission(id) {
  const r = await api('/api/mission/' + encodeURIComponent(id) + '/check', { method: 'POST' });
  const missing = r.missing
    .map(
      (l) => `<tr><td><span class="badge ${l.crit}">${l.crit.toUpperCase()}</span></td>
      <td>${l.sku}</td><td class="qty">${l.have}/${l.have + l.need}</td>
      <td class="muted">${l.sources.map((s) => s[0] + ' (' + s[1] + ')').join(', ') || 'SIN STOCK EN TALLER'}</td></tr>`
    )
    .join('');
  const route = r.route
    .map(
      (leg) => `<div class="card"><strong>${leg[0]}</strong><br>${leg[1]
        .map((x) => `☐ ${x[1]}× ${x[0]}`)
        .join('<br>')}</div>`
    )
    .join('');
  $('#missiondetail').innerHTML = `
    <div class="card">
      <h2>${r.mission} — ${r.ready ? '✅ sin bloqueantes' : '⛔ NO LISTO'}</h2>
      ${missing ? `<table><tr><th></th><th>SKU</th><th>hay/necesita</th><th>coger de</th></tr>${missing}</table>` : '<p>Carro completo.</p>'}
      ${route ? `<h2>Ruta de recogida</h2>${route}` : ''}
    </div>`;
}
window.checkMission = checkMission;

async function readyMission(id) {
  try {
    await api('/api/mission/' + encodeURIComponent(id) + '/ready', {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify({ actor: ACTOR }),
    });
    toast('Misión LISTA ✓');
    loadMissions();
  } catch (e) {
    if (e.status === 409 && e.data.blockers) {
      toast('⛔ Bloqueantes: ' + e.data.blockers.map((b) => b.sku).join(', '), 5000);
    } else {
      toast('Error: ' + e.message);
    }
  }
}
window.readyMission = readyMission;

// ---------- init ----------
loadSummary();
loadStock();
setInterval(loadSummary, 30000);
