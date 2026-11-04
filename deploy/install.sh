#!/bin/sh
# SPDX-License-Identifier: GPL-3.0-or-later
# Installs lab-agent as a systemd service running as its own unprivileged user. Run as root (or with sudo) once per machine.
#
#   deploy/install.sh
#
# Afterwards: put the two deploy keys and a filled-in config.toml under /etc/lab-agent/ (see config.example.toml), add the
# lab-agent user to whatever groups its jobs need for device access (e.g. `usermod -aG plugdev lab-agent`), then
# `systemctl enable --now lab-agent`.
set -eu

HERE=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
USER_NAME=lab-agent

if [ "$(id -u)" -ne 0 ]; then
  echo "install.sh: run as root" >&2
  exit 1
fi

if ! id "$USER_NAME" >/dev/null 2>&1; then
  useradd --system --create-home --home-dir /var/lib/lab-agent --shell /usr/sbin/nologin "$USER_NAME"
  echo "created system user $USER_NAME"
fi

install -d -o "$USER_NAME" -g "$USER_NAME" -m 0750 /etc/lab-agent
if [ ! -f /etc/lab-agent/config.toml ]; then
  install -o "$USER_NAME" -g "$USER_NAME" -m 0640 "$HERE/deploy/config.example.toml" /etc/lab-agent/config.toml
  echo "wrote /etc/lab-agent/config.toml from the example: EDIT IT before starting the service"
fi

pip3 install --break-system-packages -e "$HERE" >/dev/null
install -m 0644 "$HERE/deploy/lab-agent.service" /etc/systemd/system/lab-agent.service
systemctl daemon-reload

echo "installed. Next: put your deploy keys and a real config.toml under /etc/lab-agent/, then:"
echo "  usermod -aG plugdev $USER_NAME   # or whatever group your job scripts need for device access"
echo "  systemctl enable --now lab-agent"
echo "  journalctl -u lab-agent -f       # to watch it start"
