#!/bin/bash
# Download this fork temporarily; the installed app never uses this checkout.
set -euo pipefail

if [[ ${1:-} == --help || ${1:-} == -h ]]; then
  cat <<'USAGE'
Install or update the Spotlight fork of OpenLogi on macOS.

  bash run.sh [--destination /Applications/OpenLogi.app]

Requires Homebrew, full Xcode 26+ with its Metal toolchain, and a local
code-signing certificate. Existing app and helper identities are preserved.
OPENLOGI_REF selects a published branch or tag (default: feat/spotlight-presenter).
OPENLOGI_LOCAL_CODESIGN_IDENTITY explicitly selects a local certificate.
The downloaded source and all build outputs are removed on exit.
USAGE
  exit 0
fi

if [[ $(uname -s) != Darwin ]]; then
  echo 'This installer requires macOS.' >&2
  exit 1
fi
if [[ $EUID == 0 ]]; then
  echo 'Run this as your normal macOS user, without sudo.' >&2
  exit 1
fi
if ! command -v brew >/dev/null; then
  echo 'Install Homebrew from https://brew.sh first.' >&2
  exit 1
fi
if ! xcrun --find metal >/dev/null 2>&1; then
  echo 'Select full Xcode 26+ and install its Metal toolchain first.' >&2
  exit 1
fi

# Install missing build tools through Homebrew. User toolchains and registry
# caches remain shared; the source checkout and Cargo target directory do not.
if ! command -v cargo >/dev/null; then
  brew install rust
fi
if ! command -v cargo-bundle >/dev/null; then
  brew install cargo-bundle
fi

work=$(mktemp -d "${TMPDIR:-/tmp}/openlogi-run.XXXXXX")
cleanup() {
  local status=$?
  trap - EXIT
  rm -rf -- "$work"
  exit "$status"
}
trap cleanup EXIT
trap 'exit 130' INT
trap 'exit 143' TERM
trap 'exit 129' HUP

ref=${OPENLOGI_REF:-feat/spotlight-presenter}
echo "Downloading OpenLogi ($ref)…"
git clone --quiet --depth 1 --single-branch --branch "$ref" \
  https://github.com/andreizb/OpenLogi.git "$work/source"
cd "$work/source"
echo "Source revision: $(git rev-parse HEAD)"

# The bundler and installer share the repository's target directory. Ignore an
# inherited CARGO_TARGET_DIR so no build output escapes the disposable folder.
export CARGO_TARGET_DIR="$work/source/target"
export CARGO_INCREMENTAL=0
cargo run --locked -p xtask -- macos install "$@"
echo 'Installed and verified. Removing temporary source and build files.'
