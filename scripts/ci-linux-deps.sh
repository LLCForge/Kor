#!/usr/bin/env bash
# System libraries the Linux build links (the same list as [dist.dependencies.apt] in
# backend/dist-workspace.toml): screen capture (Wayland, GBM/DRM, PipeWire via bindgen) and D-Bus.
set -euo pipefail
sudo apt-get update
sudo apt-get install -y pkg-config libwayland-dev libgbm-dev libdrm-dev libpipewire-0.3-dev \
  libspa-0.2-dev libclang-dev clang libdbus-1-dev libegl-dev libxdo-dev
