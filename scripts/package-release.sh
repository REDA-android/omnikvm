#!/usr/bin/env bash
set -euo pipefail

OUT_DIR="${1:-dist}"
STAGE_DIR="$OUT_DIR/omnikvm-linux-x86_64"
rm -rf "$STAGE_DIR"
mkdir -p "$STAGE_DIR"

echo "==> Building static musl Rust binaries..."
CARGO_TARGET_X86_64_UNKNOWN_LINUX_MUSL_LINKER=musl-gcc \
  cargo build --release --target x86_64-unknown-linux-musl -p kvm-cli -p kvm-tray-ui

cp target/x86_64-unknown-linux-musl/release/kvm-cli "$STAGE_DIR/omnikvm"
cp target/x86_64-unknown-linux-musl/release/kvm-tray-ui "$STAGE_DIR/omnikvm-ui"
strip "$STAGE_DIR/omnikvm" "$STAGE_DIR/omnikvm-ui" 2>/dev/null || true

echo "==> Building Go mDNS beacon helper..."
(cd services/omnikvm-mdns-go && CGO_ENABLED=0 go build -ldflags="-s -w" -o "../../$STAGE_DIR/omnikvm-mdns" .)

cp README.md "$STAGE_DIR/"
tar -czf "$OUT_DIR/omnikvm-linux-x86_64.tar.gz" -C "$OUT_DIR" omnikvm-linux-x86_64
sha256sum "$OUT_DIR/omnikvm-linux-x86_64.tar.gz" > "$OUT_DIR/SHA256SUMS"

ls -lh "$STAGE_DIR" "$OUT_DIR/omnikvm-linux-x86_64.tar.gz"
cat "$OUT_DIR/SHA256SUMS"
