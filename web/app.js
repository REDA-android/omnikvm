const screens = [
  { id: "Linux-Host-4K", x: 30, y: 60, w: 190, h: 120, res: "3840x2160 @ 2.0x" },
  { id: "Win11-Work-FHD", x: 230, y: 90, w: 145, h: 90, res: "1920x1080 @ 1.0x" },
  { id: "Android-Tablet", x: 385, y: 110, w: 95, h: 70, res: "BLE HID / scrcpy" }
];

const stage = document.getElementById("topology-stage");
const readout = document.getElementById("topology-readout");
const pinEl = document.getElementById("pairing-pin");
const uriEl = document.getElementById("pairing-uri");
const qrContainer = document.getElementById("qr-container");

function renderScreens(selectedId) {
  stage.innerHTML = "";
  screens.forEach((scr) => {
    const el = document.createElement("button");
    el.type = "button";
    el.className = "screen-node" + (scr.id === selectedId ? " active" : "");
    el.style.left = scr.x + "px";
    el.style.top = scr.y + "px";
    el.style.width = scr.w + "px";
    el.style.height = scr.h + "px";
    el.innerHTML = `<strong>${scr.id}</strong><span>${scr.res}</span>`;
    el.addEventListener("click", () => {
      renderScreens(scr.id);
      readout.textContent = `Selected ${scr.id} (${scr.res}) — Hysteresis 4px, Corner Dead-Zone 16px.`;
    });
    stage.appendChild(el);
  });
}

function renderMiniQrMatrix(seed) {
  const size = 7;
  let cells = "";
  for (let y = 0; y < size; y++) {
    for (let x = 0; x < size; x++) {
      const bit = ((seed >> ((y * size + x) % 24)) ^ (x * 7 + y * 13)) & 1;
      if (bit) {
        cells += `<rect x="${x * 10 + 9}" y="${y * 10 + 9}" width="8" height="8" fill="#0f172a"/>`;
      }
    }
  }
  qrContainer.innerHTML = `<svg width="88" height="88" viewBox="0 0 88 88" role="img" aria-label="QR Code">${cells}</svg>`;
}

function regeneratePin() {
  const n = Math.floor(100000 + Math.random() * 900000);
  const str = String(n);
  pinEl.textContent = str.slice(0, 3) + " " + str.slice(3);
  uriEl.textContent = `omnikvm://pair?id=host-1&port=4433&pin=${str}`;
  renderMiniQrMatrix(n);
}

document.getElementById("regen-pin-btn").addEventListener("click", regeneratePin);
document.getElementById("reset-layout-btn").addEventListener("click", () => renderScreens(screens[0].id));

renderScreens(screens[0].id);
renderMiniQrMatrix(482910);
