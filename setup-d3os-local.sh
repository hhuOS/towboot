#!/usr/bin/env zsh
set -euo pipefail

TOWBOOT_DIR="/Users/simonkraemer/fun/towboot"
D3OS_DIR="/Users/simonkraemer/fun/D3OS"
TOWBOOTCTL_BIN="${TOWBOOT_DIR}/target/debug/towbootctl"
D3OS_TOWBOOTCTL_LINK="${D3OS_DIR}/towbootctl"

if [[ ! -d "${D3OS_DIR}" ]]; then
  print -u2 "D3OS directory not found: ${D3OS_DIR}"
  exit 1
fi

print "Building local towboot for AArch64 UEFI..."
cargo build -p towboot --target aarch64-unknown-uefi --manifest-path "${TOWBOOT_DIR}/Cargo.toml"

print "Building local towbootctl..."
cargo build -p towbootctl --features binary --manifest-path "${TOWBOOT_DIR}/Cargo.toml"

print "Linking D3OS/towbootctl to local towbootctl..."
ln -sf "${TOWBOOTCTL_BIN}" "${D3OS_TOWBOOTCTL_LINK}"

print "Done. D3OS now uses your local towbootctl via: ${D3OS_TOWBOOTCTL_LINK}"
print "You can keep using your normal D3OS commands, for example:"
print "  cd ${D3OS_DIR} && cargo make --no-workspace image"
