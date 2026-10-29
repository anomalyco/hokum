#!/bin/sh
# Builds the Hokum Linux release executable for this machine's architecture,
# dist/hokum-<version>-linux-<x64|arm64>, from a clean checkout, and prints
# its path. The Release workflow (.github/workflows/release.yml) publishes it
# as the linux-<x64|arm64> target of the release service (docs/releasing.md),
# scripts/package-linux.sh wraps it in the .deb and .rpm, and the AUR package
# hokum-bin installs it.
#
# Environment: HOKUM_VERSION (default the version in Cargo.toml; Release sets
# 0.0.0-dev-<run> for dev builds), HOKUM_RELEASE_DIST (default
# <repository>/dist), CARGO_TARGET_DIR.
set -eu

root=$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd)
dist=${HOKUM_RELEASE_DIST:-"$root/dist"}
target_dir=${CARGO_TARGET_DIR:-"$root/target"}

for command in cargo file git jq; do
  if ! command -v "$command" >/dev/null 2>&1; then
    echo "Linux builds require $command." >&2
    exit 1
  fi
done
case "$(uname -s) $(uname -m)" in
  "Linux x86_64") label=x64 elf='x86-64' ;;
  "Linux aarch64") label=arm64 elf='ARM aarch64' ;;
  *)
    echo "Linux releases must be built on x86_64 or aarch64 Linux." >&2
    exit 1
    ;;
esac
version=${HOKUM_VERSION:-$(cargo metadata --no-deps --format-version 1 --manifest-path "$root/Cargo.toml" |
  jq -r '.packages[] | select(.name == "hokum") | .version')}
if ! printf '%s\n' "$version" | grep -Eq '^([0-9]+\.[0-9]+\.[0-9]+(-beta\.[0-9]+)?|0\.0\.0-dev-[0-9]+)$'; then
  echo "Release builds need a version like 0.1.0, 0.2.0-beta.1, or 0.0.0-dev-3, not $version." >&2
  exit 1
fi
if [ -n "$(git -C "$root" status --porcelain)" ]; then
  echo "Commit or remove all working-tree changes before building a release." >&2
  exit 1
fi

"$root/scripts/set-crate-version.sh" "$root" "$version" >&2
(cd "$root" && cargo build --locked --release -p hokum >&2)
mkdir -p "$dist"
executable="$dist/hokum-$version-linux-$label"
install -m755 "$target_dir/release/hokum" "$executable"
file "$executable" | grep -E "ELF 64-bit.*$elf" >/dev/null
test "$("$executable" --version)" = "hokum $version"

# Headless renders of the cards and the settings window: proves the executable
# starts, finds a Vulkan device (lavapipe in CI), and draws, without a
# compositor or D-Bus.
temporary=$(mktemp -d)
trap 'rm -rf "$temporary"' EXIT HUP INT TERM
XDG_CONFIG_HOME="$temporary" "$executable" service --snap "$temporary/cards.png" --scene demo >&2
XDG_CONFIG_HOME="$temporary" "$executable" --snap "$temporary/settings.png" --scene hokum >&2
XDG_CONFIG_HOME="$temporary" "$executable" --snap "$temporary/sound.png" --scene sound >&2
XDG_CONFIG_HOME="$temporary" "$executable" --snap "$temporary/panel.png" --scene panel --size 364x440 >&2
file "$temporary/cards.png" "$temporary/settings.png" "$temporary/sound.png" "$temporary/panel.png" | grep -c 'PNG image data' | grep -qx 4
echo "$executable"
