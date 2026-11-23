const SCALE = 0.085;
let screens = [];
let rttHistory = [];
let focusedId = "Linux-Host-4K";
let cursorLocked = false;

const stage = document.getElementById("topology-stage");
const readout = document.getElementById("topology-readout");
const statusText = document.getElementById("status-text");
const sessionDot = document.getElementById("session-dot");
const sessionBanner = document.getElementById("session-banner");
const stopBtn = document.getElementById("stop-session-btn");
const toggleRemoteBtn = document.getElementById("toggle-remote-btn");
const syncClipBtn = document.getElementById("sync-clip-btn");
const testMoveBtn = document.getElementById("test-move-btn");
const rttBadge = document.getElementById("rtt-badge");
const chartEl = document.getElementById("latency-chart");
const lockBtn = document.getElementById("lock-cursor-btn");
const localIpHint = document.getElementById("local-ip-hint");

function renderSession(s) {
  const active = s.mode !== "idle";
  stopBtn.hidden = !active;
  syncClipBtn.hidden = !active;
  toggleRemoteBtn.hidden = s.mode !== "client";
  testMoveBtn.disabled = s.mode !== "client";
  sessionDot.classList.toggle("live", active);

  if (s.mode === "host") {
    const addr = s.lan_addr || s.bind_addr;
    statusText.textContent = `Réception active · ${addr}`;
    sessionBanner.className = "session-banner ok";
    sessionBanner.textContent =
      `Ce PC attend/reçoit sur ${addr} · Code PIN : ${s.pin} · ` +
      `${s.events_received} action(s) reçue(s) · ${s.clips_synced} presse-papiers sync`;
  } else if (s.mode === "client") {
    const where = s.remote_active ? "SUR LE PC DISTANT" : "Sur ce PC (glissez au bord pour passer)";
    toggleRemoteBtn.textContent = s.remote_active ? "Revenir sur ce PC" : "Basculer vers PC distant";
    statusText.textContent = `${where} · ${s.peer_addr}`;
    sessionBanner.className = "session-banner ok";
    sessionBanner.textContent =
      `Connecté à ${s.peer_addr} (bord: ${s.edge}) · État: ${where} · ` +
      `${s.events_sent} action(s) envoyée(s) · ${s.clips_synced} presse-papiers sync · Ctrl+Alt+Esc = revenir`;
  } else {
    statusText.textContent = "Inactif";
    sessionBanner.className = s.last_error ? "session-banner err" : "session-banner";
    sessionBanner.textContent = s.last_error
      ? `Erreur : ${s.last_error}`
      : "Aucune session active. Cliquez sur « Partager ce PC » (PC cible) ou « Connecter & Activer » (PC maître).";
  }
}

async function pollSession() {
  try {
    const res = await fetch("/api/session");
    if (res.ok) renderSession(await res.json());
  } catch (_) {}
}

document.getElementById("start-host-btn").addEventListener("click", async () => {
  const port = parseInt(document.getElementById("host-port").value, 10) || 4433;
  const pin = document.getElementById("host-pin").value.trim() || null;
  const res = await fetch("/api/session/start-host", {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ port, pin }),
  });
  const d = await res.json();
  if (d.ok) {
    if (d.session.pin) document.getElementById("host-pin").value = d.session.pin;
    renderSession(d.session);
  } else {
    sessionBanner.className = "session-banner err";
    sessionBanner.textContent = `Erreur : ${d.error}`;
  }
});

document.getElementById("start-client-btn").addEventListener("click", async () => {
  const peer = document.getElementById("client-peer").value.trim();
  const pin = document.getElementById("client-pin").value.trim();
  const edge = document.getElementById("client-edge").value;
  sessionBanner.className = "session-banner";
  sessionBanner.textContent = `Connexion QUIC chiffrée vers ${peer}…`;
  const res = await fetch("/api/session/start-client", {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ peer, pin, edge, capture: true }),
  });
  const d = await res.json();
  if (d.ok) {
    renderSession(d.session);
  } else {
    sessionBanner.className = "session-banner err";
    sessionBanner.textContent = `Erreur : ${d.error}`;
  }
});

toggleRemoteBtn.addEventListener("click", async () => {
  const res = await fetch("/api/session/toggle-remote", { method: "POST" });
  const d = await res.json();
  if (d.ok) renderSession(d.session);
});

syncClipBtn.addEventListener("click", async () => {
  const res = await fetch("/api/session/send-clipboard", {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({}),
  });
  const d = await res.json();
  if (d.ok) {
    renderSession(d.session);
  } else {
    sessionBanner.className = "session-banner err";
    sessionBanner.textContent = `Presse-papiers : ${d.error}`;
  }
});

testMoveBtn.addEventListener("click", async () => {
  const res = await fetch("/api/session/test-move", {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ dx: 40, dy: 0 }),
  });
  const d = await res.json();
  if (d.ok) renderSession(d.session);
});

stopBtn.addEventListener("click", async () => {
  const res = await fetch("/api/session/stop", { method: "POST" });
  const d = await res.json();
  if (d.ok) renderSession(d.session);
});

function renderScreens() {
  stage.innerHTML = "";
  screens.forEach((scr) => {
    const el = document.createElement("div");
    el.className = "screen-box" + (scr.id === focusedId ? " focused" : "");
    el.style.left = `${20 + scr.x * SCALE}px`;
    el.style.top = `${20 + scr.y * SCALE}px`;
    el.style.width = `${Math.max(96, scr.width * SCALE)}px`;
    el.style.height = `${Math.max(66, scr.height * SCALE)}px`;
    el.innerHTML = `<div class="screen-title">${scr.id}</div><div class="screen-meta">${scr.width}x${scr.height} · ${scr.dpi_scale.toFixed(1)}x</div>`;

    let dragging = false, startX = 0, startY = 0, origX = 0, origY = 0;
    el.addEventListener("pointerdown", (e) => {
      dragging = true;
      startX = e.clientX;
      startY = e.clientY;
      origX = scr.x;
      origY = scr.y;
      focusedId = scr.id;
      el.setPointerCapture(e.pointerId);
    });
    el.addEventListener("pointermove", (e) => {
      if (!dragging) return;
      scr.x = Math.round((origX + (e.clientX - startX) / SCALE) / 10) * 10;
      scr.y = Math.round((origY + (e.clientY - startY) / SCALE) / 10) * 10;
      el.style.left = `${20 + scr.x * SCALE}px`;
      el.style.top = `${20 + scr.y * SCALE}px`;
      readout.textContent = `${scr.id} → (${scr.x}, ${scr.y})`;
    });
    el.addEventListener("pointerup", () => {
      dragging = false;
      renderScreens();
    });
    stage.appendChild(el);
  });
}

function renderLatencyChart() {
  if (rttHistory.length < 2) return;
  const w = 600, h = 72;
  const maxVal = Math.max(10, ...rttHistory) * 1.2;
  const pts = rttHistory
    .map((val, i) => {
      const x = (i / Math.max(1, rttHistory.length - 1)) * (w - 16) + 8;
      const y = h - 8 - Math.min(1, val / maxVal) * (h - 16);
      return `${x.toFixed(1)},${y.toFixed(1)}`;
    })
    .join(" ");
  chartEl.innerHTML = `<polyline fill="none" stroke="#0f766e" stroke-width="2.5" points="${pts}" />`;
}

async function loadTopology() {
  screens = await (await fetch("/api/topology")).json();
  renderScreens();
}

async function saveTopology() {
  const res = await fetch("/api/topology", {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify(screens),
  });
  const d = await res.json();
  readout.textContent = `Disposition enregistrée (${d.saved} écrans).`;
}

async function loadPairing() {
  const data = await (await fetch("/api/pairing")).json();
  document.getElementById("pairing-pin").textContent = data.pin;
  document.getElementById("pairing-uri").textContent = data.uri;
  document.getElementById("qr-container").innerHTML = data.qr_svg;
  const hostPin = document.getElementById("host-pin");
  if (!hostPin.value) hostPin.value = data.pin;
}

function renderLock(locked) {
  cursorLocked = locked;
  lockBtn.textContent = "Verrouillage curseur : " + (locked ? "ON" : "OFF");
}

lockBtn.addEventListener("click", async () => {
  const res = await fetch("/api/cursor-lock", {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ locked: !cursorLocked }),
  });
  renderLock((await res.json()).locked);
});

async function loadHealthAndPeers() {
  try {
    const [hRes, pRes] = await Promise.all([fetch("/api/health"), fetch("/api/peers")]);
    const health = await hRes.json();
    const peers = await pRes.json();
    const port = document.getElementById("host-port").value || "4433";
    localIpHint.innerHTML = `Adresse réseau de ce PC : <strong>${peers.local_ip}:${port}</strong> (${health.hostname})`;
    const mb = health.rss_kb ? `${(health.rss_kb / 1024).toFixed(1)} Mo` : health.os;
    document.getElementById("health-rss").textContent = `${health.os} · ${mb}`;
    document.getElementById("peers-readout").textContent = peers.discovered.length
      ? `${peers.discovered.length} PC trouvé(s) sur votre réseau (cliquez pour remplir l'IP) :`
      : `Ce PC (${peers.local_ip}) est visible sur le réseau. Ouvrez OmniKVM sur un 2e PC pour le voir apparaître ici.`;
    const list = document.getElementById("peers-list");
    list.replaceChildren(
      ...peers.discovered.map((p) => {
        const li = document.createElement("li");
        const ip = p.addr.split(":")[0];
        li.innerHTML = `<span><strong>${p.hostname}</strong> (${p.os})</span><span class="badge ok">${ip}:${p.quic_port}</span>`;
        li.addEventListener("click", () => {
          document.getElementById("client-peer").value = `${ip}:${p.quic_port}`;
        });
        return li;
      })
    );
  } catch (_) {}
}

function connectSse() {
  const es = new EventSource("/api/events");
  es.onmessage = (ev) => {
    const msg = JSON.parse(ev.data);
    rttBadge.textContent = `${msg.pipeline_us} µs`;
    if (msg.cursor_locked !== cursorLocked) renderLock(msg.cursor_locked);
    rttHistory.push(msg.pipeline_us);
    if (rttHistory.length > 40) rttHistory.shift();
    renderLatencyChart();
  };
}

document.getElementById("regen-pin-btn").addEventListener("click", loadPairing);
document.getElementById("save-layout-btn").addEventListener("click", saveTopology);
document.getElementById("reset-layout-btn").addEventListener("click", loadTopology);

loadTopology();
loadPairing();
loadHealthAndPeers();
pollSession();
connectSse();
setInterval(() => { loadHealthAndPeers(); pollSession(); }, 1500);
