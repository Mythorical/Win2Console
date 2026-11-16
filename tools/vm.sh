#!/usr/bin/env bash
# Run a command inside the ConsoleDev VM from the Linux host (see docs/BUILDING.md).
#
#   tools/vm.sh 'cd /d C:\cs-src && cargo test --workspace'   # runs via cmd.exe /c
#   tools/vm.sh --sync                                        # mirror Z:\ (read-only share) -> C:\cs-src
#   tools/vm.sh --cargo test --workspace                      # sync, then run cargo in C:\cs-src
#   tools/vm.sh --shot m1-home                                # screenshot -> docs/screenshots/m1-home.png
#   tools/vm.sh --pull Cargo.lock                             # copy a file from C:\cs-src back to the repo
#   tools/vm.sh --keys enter right right down esc             # send key presses to the VM (UI testing)
#   tools/vm.sh --type "Player One"                           # type text into the VM
#   tools/vm.sh --relaunch [release]                          # restart Win2Console on the VM desktop
#
# Env: CS_VM (default ConsoleDev), CS_VM_USER (default vboxuser), CS_VM_PASSFILE (required: file containing the password).
set -euo pipefail
VM="${CS_VM:-ConsoleDev}"
USER_NAME="${CS_VM_USER:-vboxuser}"
ROOT="$(cd "$(dirname "$0")/.." && pwd)"

if [[ "${1:-}" == "--shot" ]]; then
  out="$ROOT/docs/screenshots/${2:?screenshot name}.png"
  VBoxManage controlvm "$VM" screenshotpng "$out"
  echo "$out"
  exit 0
fi

if [[ "${1:-}" == "--keys" ]]; then
  shift
  declare -A CODES=(
    [enter]="1c 9c" [esc]="01 81" [backspace]="0e 8e" [space]="39 b9"
    [up]="e0 48 e0 c8" [down]="e0 50 e0 d0" [left]="e0 4b e0 cb" [right]="e0 4d e0 cd"
    [pgup]="e0 49 e0 c9" [pgdn]="e0 51 e0 d1" [home]="e0 47 e0 c7"
    [x]="2d ad" [y]="15 95" [m]="32 b2" [v]="2f af"
  )
  for k in "$@"; do
    codes="${CODES[$k]:?unknown key $k}"
    # shellcheck disable=SC2086
    VBoxManage controlvm "$VM" keyboardputscancode $codes
    sleep "${CS_KEY_DELAY:-0.4}"
  done
  exit 0
fi

if [[ "${1:-}" == "--type" ]]; then
  VBoxManage controlvm "$VM" keyboardputstring "${2:?text}"
  exit 0
fi

if [[ "${1:-}" == "--pull" ]]; then
  : "${CS_VM_PASSFILE:?set CS_VM_PASSFILE}"
  rel="${2:?relative path}"
  VBoxManage guestcontrol "$VM" copyfrom --username "$USER_NAME" --passwordfile "$CS_VM_PASSFILE" \
    "C:\\cs-src\\${rel//\//\\}" "$ROOT/$rel"
  exit 0
fi

: "${CS_VM_PASSFILE:?set CS_VM_PASSFILE to a file containing the VM user password}"
# The share is read-only, so builds run on a mirror. robocopy exit codes < 8 mean success.
ROBO='robocopy Z:\ C:\cs-src /MIR /NFL /NDL /NJH /NJS /NP /XD target .git Z:\playnite-extension\bin Z:\playnite-extension\obj /XF Cargo.lock *.zip >nul'
# In cmd, everything after `if ... (...)` on the same line belongs to the if, so use an explicit else.
SYNC="$ROBO & if errorlevel 8 (echo SYNC FAILED) else (echo synced)"
case "${1:-}" in
  --sync) set -- "$SYNC" ;;
  --relaunch)
    # Processes started by guestcontrol live on a hidden window station, so ask the desktop's Explorer to
    # start the app on the visible desktop. (Scheduled Tasks worked at first but get stuck "Queued".)
    set -- "taskkill /im Win2Console.exe /f >nul 2>&1 & explorer.exe C:\\cs-target\\${2:-debug}\\Win2Console.exe" ;;
  --cargo) shift; set -- "$ROBO & if errorlevel 8 (echo SYNC FAILED) else (cd /d C:\cs-src && cargo $* 2>&1)" ;;
esac

# Arguments after `--` are argv[1..] in VirtualBox 7, so pass only the cmd.exe flags.
exec VBoxManage guestcontrol "$VM" run --username "$USER_NAME" --passwordfile "$CS_VM_PASSFILE" \
  --exe 'C:\Windows\System32\cmd.exe' --timeout "${CS_VM_TIMEOUT:-3600000}" -- /c "$1"
