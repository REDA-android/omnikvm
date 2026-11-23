const SCALE = 0.065;
let screens = [];
let selectedId = "Linux-Host-4K";
let focusedId = "Linux-Host-4K";
const rttHistory = [];

const stage = document.getElementById("topology-stage");
const readout = document.getElementById("topology-readout");
const pinEl = document.getElementById("pairing-pin");
const uriEl = document.getElementById("pairing-uri");
const qrContainer = document.getElementById("qr-container");
const statusText = document.getElementById("status-text");
const rttBadge = document.getElementById("rtt-badge");
const chartEl = document.getElementById("latency-chart");

function renderScreens() {
  stage.innerHTML = "";
  screens.forEach((scr) => {
    const el = document.createElement("div");
    el.className =
      "screen-node" +
      (scr.id === selectedId ? " active" : "") +
      (scr.id === focusedId ? " focused" : "");
    el.style.left = Math.max(8, Math.round(scr.x * SCALE) + 16) + "px";
    el.style.top = Math.max(8, Math.round(scr.y * SCALE) + 24) + "px";
    el.style.width = Math.max(82, Math.round(scr.width * SCALE)) + "px";
    el.style.height = Math.max(58, Math.round(scr.height * SCALE)) + "px";
    el.innerHTML = `<strong>${scr.id}</strong><span>${scr.width}x${scr.height} @ ${scr.dpi_scale}x</span>`;

    let dragging = false;
    let startX = 0;
    let startY = 0;
    let origX = scr.x;
    let origY = scr.y;

    el.addEventListener("pointerdown", (e) => {
      dragging = true;
      selectedId = scr.id;
      startX = e.clientX;
      startY = e.clientY;
      origX = scr.x;
      origY = scr.y;
      el.setPointerCapture(e.pointerId);
      readout.textContent = `Selected ${scr.id} (${scr.width}x${scr.height} @ ${scr.dpi_scale}x) at (${scr.x}, ${scr.y}).`;
    });

    el.addEventListener("pointermove", (e) => {
      if (!dragging) return;
      scr.x = Math.max(0, Math.round(origX + (e.clientX - startX) / SCALE));
      scr.y = Math.max(0, Math.round(origY + (e.clientY - startY) / SCALE));
      el.style.left = Math.max(8, Math.round(scr.x * SCALE) + 16) + "px";
      el.style.top = Math.max(8, Math.round(scr.y * SCALE) + 24) + "px";
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
  const w = 600;
  const h = 90;
  const maxRtt = 1.0;
  const pts = rttHistory
    .map((val, i) => {
      const x = (i / Math.max(1, rttHistory.length - 1)) * (w - 16) + 8;
      const y = h - 10 - Math.min(1, val / maxRtt) * (h - 20);
      return `${x.toFixed(1)},${y.toFixed(1)}`;
    })
    .join(" ");
  chartEl.innerHTML = `<polyline fill="none" stroke="#0f766e" stroke-width="2.5" points="${pts}" />`;
}

async function loadTopology() {
  const res = await fetch("/api/topology");
  screens = await res.json();
  renderScreens();
}

async function saveTopology() {
  const res = await fetch("/api/topology", {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify(screens),
  });
  const data = await res.json();
  readout.textContent = `Saved ${data.saved} screen(s) to live OmniKVM topology.`;
}

async function loadPairing() {
  const res = await fetch("/api/pairing");
  const data = await res.json();
  pinEl.textContent = data.pin.slice(0, 3) + " " + data.pin.slice(3);
  uriEl.textContent = data.uri;
  qrContainer.innerHTML = data.qr_svg;
}

async function runTransferTest() {
  const el = document.getElementById("transfer-result");
  el.textContent = "Encrypting & verifying 2 MB Blake3-XOF stream...";
  const res = await fetch("/api/transfer-test", {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ size_kb: 2048 }),
  });
  const data = await res.json();
  el.textContent = `Verified ${data.chunks} x 64KB chunks in ${(data.elapsed_us / 1000).toFixed(2)} ms (blake3: ${data.blake3_hex}...)`;
}

function connectSse() {
  const es = new EventSource("/api/events");
  es.onmessage = (ev) => {
    const msg = JSON.parse(ev.data);
    statusText.textContent = `${msg.transport} · ${msg.rtt_ms} ms`;
    rttBadge.textContent = `${msg.rtt_ms} ms`;
    if (msg.active_screen !== focusedId) {
      focusedId = msg.active_screen;
      renderScreens();
    }
    rttHistory.push(msg.rtt_ms);
    if (rttHistory.length > 40) rttHistory.shift();
    renderLatencyChart();
  };
}

let cursorLocked = false;
document.getElementById("lock-cursor-btn").addEventListener("click", (e) => {
  cursorLocked = !cursorLocked;
  e.target.textContent = "Lock Cursor: " + (cursorLocked ? "ON" : "OFF");
  readout.textContent = cursorLocked
    ? "Cursor locked to active screen (gaming/fullscreen mode)."
    : "Cursor edge switching enabled (hysteresis 4px, dead-zone 16px).";
});

async function loadHealthAndPeers() {
  try {
    const [hRes, pRes] = await Promise.all([fetch("/api/health"), fetch("/api/peers")]);
    const health = await hRes.json();
    const peers = await pRes.json();
    const mb = health.rss_kb ? (health.rss_kb / 1024).toFixed(1) + " MB" : "N/A";
    document.getElementById("health-rss").textContent = `${health.os} · ${mb}`;
    document.getElementById("peers-readout").textContent =
      `${peers.service} — local node ${peers.local.peer_id} (QUIC :${peers.local.quic_port})`;
  } catch (_) {}
}

document.getElementById("regen-pin-btn").addEventListener("click", loadPairing);
document.getElementById("save-layout-btn").addEventListener("click", saveTopology);
document.getElementById("reset-layout-btn").addEventListener("click", loadTopology);
document.getElementById("transfer-btn").addEventListener("click", runTransferTest);

loadTopology();
loadPairing();
loadHealthAndPeers();
connectSse();
