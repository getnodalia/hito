# shellcheck shell=bash
# Shared setup for the packaging scripts. Source it: `. "$(dirname "$0")/../env.sh"`.
# The release workflows (.github/workflows/release-*.yml) run the scripts that source it.
#
# Exports:
#   ROOT               workspace root
#   VERSION            [workspace.package] version from Cargo.toml (override: HITO_VERSION)
#   DIST               output directory for release files (default: $ROOT/dist/release)
#   HITO_BUILD_SHA    git commit of the build (Info.plist)
#   HITO_BUILD_DATE   UTC build date, YYYY-MM-DD (AppStream release date)
#   CARGO_TARGET_DIR   cargo's target dir (default: $ROOT/target)
#   APP_ID             dev.danielmiguelt.Hito

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
export ROOT

# The version lives in exactly one place: `[workspace.package] version` in the root Cargo.toml.
workspace_version() {
  awk '
    /^\[/ { in_pkg = ($0 == "[workspace.package]") ; next }
    in_pkg && $1 == "version" { gsub(/[" \r]/, "", $3); print $3; exit }
  ' "$ROOT/Cargo.toml"
  return 0
}

VERSION="${HITO_VERSION:-$(workspace_version)}"
if [[ -z "$VERSION" ]]; then
  echo "error: could not read [workspace.package] version from $ROOT/Cargo.toml" >&2
  exit 1
fi
export VERSION

DIST="${DIST:-$ROOT/dist/release}"
mkdir -p "$DIST"
export DIST

if [[ -z "${HITO_BUILD_SHA:-}" ]]; then
  HITO_BUILD_SHA="$(git -C "$ROOT" rev-parse HEAD 2>/dev/null || true)"
fi
export HITO_BUILD_SHA
export HITO_BUILD_DATE="${HITO_BUILD_DATE:-$(date -u +%Y-%m-%d)}"
export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-$ROOT/target}"

# One id for the desktop entry, AppStream, icons, the macOS bundle id and the window's app_id.
export APP_ID=dev.danielmiguelt.Hito

# Emit a GitHub Actions warning (plain stderr outside Actions).
warn() {
  if [[ -n "${GITHUB_ACTIONS:-}" ]]; then echo "::warning::$*"; else echo "warning: $*" >&2; fi
  return 0
}

# Copy the licence, notice and readme files, and the example server configuration.
copy_docs() {
  local dest="$1" f
  for f in README.md LICENSE-MIT LICENSE-APACHE NOTICE ATTRIBUTION.md hito.example.toml; do
    if [[ -f "$ROOT/$f" ]]; then cp "$ROOT/$f" "$dest/"; fi
  done
  return 0
}

# Portable SHA-256 of a file (prints just the hash).
sha256() {
  local file="$1"
  if command -v sha256sum >/dev/null; then sha256sum "$file" | cut -d' ' -f1; else shasum -a 256 "$file" | cut -d' ' -f1; fi
  return 0
}
