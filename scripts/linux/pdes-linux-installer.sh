#!/usr/bin/env bash

set -euo pipefail

INSTALL_ROOT="/opt/pdes-win-fw-updater"
BOSSAC_PATH="${INSTALL_ROOT}/tools/bossac"
UDEV_RULES_FILE="/etc/udev/rules.d/99-pdes.rules"

echo "Configuring PDES Linux dependencies (udev rules)..."
echo "Note: bossac is expected to be bundled with PDES at ${BOSSAC_PATH}; no system-wide bossac install is performed."

# Must be root
if [[ $EUID -ne 0 ]]; then
  echo "Please run as root (use sudo)" >&2
  exit 1
fi

if [[ -f /lib/udev/rules.d/99-pdes.rules ]]; then
  echo "PDES udev rules already installed at /lib/udev/rules.d/99-pdes.rules"
else
  echo "Installing PDES udev rules to ${UDEV_RULES_FILE}..."
  cat > "$UDEV_RULES_FILE" << 'EOF'
# Arduino / Atmel SAM / SAMD boards (bossac)
SUBSYSTEM=="tty", ATTRS{idVendor}=="2341", MODE="0666", GROUP="dialout"
SUBSYSTEM=="tty", ATTRS{idVendor}=="239a", MODE="0666", GROUP="dialout"
SUBSYSTEM=="tty", ATTRS{idVendor}=="2a03", MODE="0666", GROUP="dialout"
SUBSYSTEM=="tty", ATTRS{idVendor}=="03eb", MODE="0666", GROUP="dialout"

# Arduino bootloaders (CDC ACM)
SUBSYSTEM=="tty", ATTRS{idVendor}=="2341", ATTRS{idProduct}=="003d", MODE="0666", GROUP="dialout"
SUBSYSTEM=="tty", ATTRS{idVendor}=="2341", ATTRS{idProduct}=="004d", MODE="0666", GROUP="dialout"

# Allow access to USB devices directly
SUBSYSTEM=="usb", ATTRS{idVendor}=="2341", MODE="0666", GROUP="dialout"
SUBSYSTEM=="usb", ATTRS{idVendor}=="239a", MODE="0666", GROUP="dialout"
SUBSYSTEM=="usb", ATTRS{idVendor}=="2a03", MODE="0666", GROUP="dialout"
SUBSYSTEM=="usb", ATTRS{idVendor}=="03eb", MODE="0666", GROUP="dialout"
EOF
fi

# -------------------------
# Reload udev
# -------------------------
echo "Reloading udev rules..."
if command -v udevadm >/dev/null 2>&1; then
  udevadm control --reload-rules || true
  udevadm trigger || true
fi

# -------------------------
# Verify
# -------------------------
if [[ -x "$BOSSAC_PATH" ]]; then
  echo "Found bundled bossac at $BOSSAC_PATH"
  "$BOSSAC_PATH" --version || true
else
  echo "NOTE: bundled bossac not found at $BOSSAC_PATH" >&2
  echo "Install the PDES .deb to get bossac bundled, or place bossac at that path." >&2
fi

echo "Done."
