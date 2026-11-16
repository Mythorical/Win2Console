#!/usr/bin/env bash
# Builds a release in the VM (tools/release.cmd) and publishes it as a GitHub release, which installed copies
# of Win2Console update from.
#   tools/publish.sh 0.2.0 ["Release notes"]
# Needs CS_VM_PASSFILE (see vm.sh) and `gh` signed in to the repo's owner.
set -euo pipefail
VERSION="${1:?version, e.g. 0.2.0}"
NOTES="${2:-Win2Console $VERSION}"
HERE="$(cd "$(dirname "$0")" && pwd)"
ROOT="$(dirname "$HERE")"
: "${CS_VM_PASSFILE:?set CS_VM_PASSFILE}"

# The version must match Cargo.toml, which the app reports as its own version.
CARGO_VERSION="$(grep -m1 '^version' "$ROOT/Cargo.toml" | cut -d'"' -f2)"
[[ "$CARGO_VERSION" == "$VERSION" ]] || { echo "Cargo.toml says $CARGO_VERSION; bump it to $VERSION first"; exit 1; }

CS_VM_TIMEOUT=3600000 "$HERE/vm.sh" "C:\\cs-src\\tools\\release.cmd $VERSION" | tr -d '\r'

OUT="$(mktemp -d)"
for f in Win2Console-win-Setup.exe Win2Console-win-Portable.zip "Win2Console-$VERSION-full.nupkg" releases.win.json RELEASES assets.win.json; do
  VBoxManage guestcontrol "${CS_VM:-ConsoleDev}" copyfrom --username "${CS_VM_USER:-vboxuser}" \
    --passwordfile "$CS_VM_PASSFILE" "C:\\cs-target\\releases\\$f" "$OUT/$f" 2>/dev/null || true
done
ls -la "$OUT"
# Delta packages (only what changed since the last version), when vpk made one.
VBoxManage guestcontrol "${CS_VM:-ConsoleDev}" copyfrom --username "${CS_VM_USER:-vboxuser}" \
  --passwordfile "$CS_VM_PASSFILE" "C:\\cs-target\\releases\\Win2Console-$VERSION-delta.nupkg" \
  "$OUT/Win2Console-$VERSION-delta.nupkg" 2>/dev/null || true

gh release create "v$VERSION" "$OUT"/* --repo Mythorical/Win2Console --title "Win2Console $VERSION" --notes "$NOTES"
echo "Published v$VERSION"
