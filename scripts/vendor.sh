#!/usr/bin/env bash
set -euo pipefail

VENDOR_DIR="${1:-vendor_refs}"
mkdir -p "$VENDOR_DIR"

declare -A REPOS=(
  ["deskflow"]="https://github.com/deskflow/deskflow"
  ["input-leap"]="https://github.com/input-leap/input-leap"
  ["barrier"]="https://github.com/debauchee/barrier"
  ["lan-mouse"]="https://github.com/feschber/lan-mouse"
  ["mickey"]="https://github.com/EduardoMG12/mickey"
  ["mangue_sechee"]="https://github.com/Aquilesorei/mangue_sechee"
  ["mykvm"]="https://github.com/XxMinor/mykvm"
  ["mousehop"]="https://github.com/jondkinney/mousehop"
  ["synergy-core"]="https://github.com/raimundojimenez/synergy-core"
  ["lan-mouse-android"]="https://github.com/kiss-kedaya/lan-mouse"
  ["input-leaf"]="https://github.com/anasvhora284/input-leaf"
  ["scrcpy"]="https://github.com/Genymobile/scrcpy"
  ["rustdesk"]="https://github.com/rustdesk/rustdesk"
  ["leap-scrcpy"]="https://github.com/yume-chan/leap-scrcpy"
  ["esparrier"]="https://github.com/windoze/esparrier"
  ["enigo"]="https://github.com/enigo-rs/enigo"
  ["rdev"]="https://github.com/Narsil/rdev"
  ["evdev"]="https://github.com/emberian/evdev"
  ["robotgo"]="https://github.com/go-vgo/robotgo"
  ["enigo-zyris"]="https://github.com/ridanit-ruma/enigo-zyris"
  ["arboard"]="https://github.com/1Password/arboard"
  ["wl-clipboard-rs"]="https://github.com/YaLTeR/wl-clipboard-rs"
  ["localsend"]="https://github.com/localsend/localsend"
  ["wisp"]="https://github.com/YohannHommet/wisp"
  ["latticesend"]="https://github.com/Niumination/latticesend"
  ["webrtc-rs"]="https://github.com/webrtc-rs/webrtc"
  ["pion-webrtc"]="https://github.com/pion/webrtc"
  ["quinn"]="https://github.com/quinn-rs/quinn"
  ["kvm-switch"]="https://github.com/timgws/kvm-switch"
  ["powertoys"]="https://github.com/microsoft/PowerToys"
)

echo "==> Starting strategic cloning of ${#REPOS[@]} reference repositories..."

for name in "${!REPOS[@]}"; do
  target_path="$VENDOR_DIR/$name"
  if [ ! -d "$target_path" ]; then
    echo "Cloning $name..."
    git clone --depth 1 --single-branch "${REPOS[$name]}" "$target_path" || echo "WARN: failed to clone $name"
  else
    echo "$name already present in $VENDOR_DIR."
  fi
done

echo "==> Stripping .git directories to protect agent context..."
find "$VENDOR_DIR" -name ".git" -exec rm -rf {} + 2>/dev/null || true
echo "==> Done in $VENDOR_DIR"
