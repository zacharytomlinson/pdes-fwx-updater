#!/usr/bin/env bash

set -e

BOSSAC_VERSION="1.9.1"
BOSSAC_URL="https://github.com/shumatech/BOSSA/releases/download/${BOSSAC_VERSION}/bossac-${BOSSAC_VERSION}-x86_64-linux.tar.gz"
INSTALL_DIR="/usr/local/bossac-${BOSSAC_VERSION}"
BIN_LINK="/usr/local/bin/bossac"
UDEV_RULES_FILE="/etc/udev/rules.d/99-arduino.rules"

echo "Installing bossac ${BOSSAC_VERSION} with Arduino udev rules..."

# Must be root
if [[ $EUID -ne 0 ]]; then
  echo "Please run as root (use sudo)"
  exit 1
fi

# -------------------------
# Install bossac
# -------------------------
TMP_DIR=$(mktemp -d)
cd "$TMP_DIR"

echo "Downloading bossac..."
curl -L -o bossac.tar.gz "$BOSSAC_URL"

echo "Extracting..."
tar -xzf bossac.tar.gz

echo "Installing to ${INSTALL_DIR}..."
mkdir -p "$INSTALL_DIR"
cp bossac "$INSTALL_DIR/"
chmod +x "$INSTALL_DIR/bossac"

echo "Creating symlink ${BIN_LINK}..."
ln -sf "$INSTALL_DIR/bossac" "$BIN_LINK"

cd /
rm -rf "$TMP_DIR"

# -------------------------
# Install udev rules
# -------------------------
echo "Installing Arduino udev rules..."

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

# -------------------------
# Reload udev
# -------------------------
echo "Reloading udev rules..."
udevadm control --reload-rules
udevadm trigger

# -------------------------
# Ensure user is in dialout group
# -------------------------
TARGET_USER=${SUDO_USER:-$(logname)}

if ! id -nG "$TARGET_USER" | grep -qw dialout; then
  echo "Adding user '$TARGET_USER' to dialout group..."
  usermod -aG dialout "$TARGET_USER"
  echo "⚠️  Please log out and log back in for group changes to take effect."
fi

# -------------------------
# Verify
# -------------------------
echo "Verifying bossac installation..."
bossac --version

echo
echo "✅ bossac ${BOSSAC_VERSION} installed successfully"
echo "✅ udev rules installed: $UDEV_RULES_FILE"
echo "ℹ️  You may need to unplug and replug your Arduino"

