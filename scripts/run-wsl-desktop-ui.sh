#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$repo_root"

required_packages=(
  pkg-config
  libgtk-3-dev
  libsoup-3.0-dev
  libwebkit2gtk-4.1-dev
  libxdo-dev
)

missing_packages=()

for package in "${required_packages[@]}"; do
  if ! dpkg-query -W -f='${Status}\n' "$package" 2>/dev/null | grep -q "install ok installed"; then
    missing_packages+=("$package")
  fi
done

if ((${#missing_packages[@]} > 0)); then
  cat <<EOF
Missing WSL desktop dependencies:
  ${missing_packages[*]}

Install them with:
  sudo apt update
  sudo apt install -y ${missing_packages[*]}

Then rerun:
  ./scripts/run-wsl-desktop-ui.sh
EOF
  exit 1
fi

if [[ ! -x /usr/bin/pkg-config ]]; then
  echo "Expected /usr/bin/pkg-config to exist and be executable."
  exit 1
fi

if [[ -z "${WAYLAND_DISPLAY:-}" && -z "${DISPLAY:-}" ]]; then
  cat <<'EOF'
No GUI display was detected in WSL.
Make sure WSLg or another Linux GUI bridge is available before launching the native desktop UI.
EOF
  exit 1
fi

export PKG_CONFIG=/usr/bin/pkg-config

exec cargo run --features desktop-ui "$@"
