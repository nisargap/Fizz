#!/bin/sh
# Fizz CLI installer — https://fizz-zeta.vercel.app/install.sh
#
# Downloads the `fizz` binary for this device from GitHub Releases
# (https://github.com/nisargap/Fizz/releases), checks its SHA-256 checksum, and installs it.
# Read it first if you like:  curl -fsSL https://fizz-zeta.vercel.app/install.sh | less
#
# Options (environment variables):
#   FIZZ_VERSION=cli-v0.1.0   install a specific release instead of the latest
#   FIZZ_INSTALL_DIR=DIR      install somewhere other than /usr/local/bin or ~/.local/bin
set -eu

REPO="nisargap/Fizz"

say() { printf '%s\n' "$*"; }
fail() { printf 'fizz install: %s\n' "$*" >&2; exit 1; }
need() { command -v "$1" >/dev/null 2>&1 || fail "This installer needs $1."; }

sha256() {
  if command -v sha256sum >/dev/null 2>&1; then sha256sum "$1" | cut -d' ' -f1
  elif command -v shasum >/dev/null 2>&1; then shasum -a 256 "$1" | cut -d' ' -f1
  else fail "This installer needs sha256sum or shasum to verify the download."; fi
}

# Everything runs inside main so a partially downloaded script never executes.
main() {
  need curl
  os=$(uname -s)
  [ "$os" = "Linux" ] || fail "The Fizz CLI runs on Linux devices; this is $os."
  case "$(uname -m)" in
    aarch64 | arm64) target="aarch64-unknown-linux-musl" ;;
    armv7l | armv8l) target="armv7-unknown-linux-musleabihf" ;;
    armv6l) target="arm-unknown-linux-musleabihf" ;;
    x86_64 | amd64) target="x86_64-unknown-linux-musl" ;;
    *) fail "There is no Fizz CLI build for $(uname -m) yet." ;;
  esac

  version="${FIZZ_VERSION:-latest}"
  if [ "$version" = "latest" ]; then
    base="https://github.com/$REPO/releases/latest/download"
  else
    base="https://github.com/$REPO/releases/download/$version"
  fi

  tmp=$(mktemp -d)
  trap 'rm -rf "$tmp"' EXIT INT TERM
  say "Downloading fizz ($target)…"
  curl -fsSL --proto '=https' --tlsv1.2 "$base/fizz-$target" -o "$tmp/fizz" || fail "Download failed. Check your connection, or set FIZZ_VERSION."
  curl -fsSL --proto '=https' --tlsv1.2 "$base/SHA256SUMS" -o "$tmp/SHA256SUMS" || fail "Could not download checksums."
  expected=$(awk -v f="fizz-$target" '$2 == f { print $1 }' "$tmp/SHA256SUMS")
  [ -n "$expected" ] || fail "No checksum is published for fizz-$target."
  [ "$(sha256 "$tmp/fizz")" = "$expected" ] || fail "Checksum mismatch. Not installing."
  chmod 755 "$tmp/fizz"

  if [ -n "${FIZZ_INSTALL_DIR:-}" ]; then dir="$FIZZ_INSTALL_DIR"
  elif [ -w /usr/local/bin ]; then dir="/usr/local/bin"
  else dir="$HOME/.local/bin"; fi
  mkdir -p "$dir"
  mv "$tmp/fizz" "$dir/fizz"

  say "Installed $("$dir/fizz" --version) to $dir/fizz"
  case ":$PATH:" in
    *":$dir:"*) ;;
    *) say "Add $dir to your PATH, for example:  echo 'export PATH=\"$dir:\$PATH\"' >> ~/.profile && . ~/.profile" ;;
  esac
  say ""
  say "Next, pair this device:  fizz pair"
}

main "$@"
