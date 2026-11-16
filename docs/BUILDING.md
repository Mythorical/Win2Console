# Building Win2Console & setting up the dev VM

The code lives on the Linux host in `~/console-project`. A clean Windows 11 VirtualBox VM
reads it through a shared folder, builds it with the MSVC Rust toolchain, and runs it. Screenshots are
taken from the host with `VBoxManage`.

> **Do not use the "Windows 11 Malware Analysis" VM.** Real launcher, Steam and Discord accounts
> get signed in during testing, so the VM has to be clean.

## 1. Create the VM (done by you)

| Setting | Value |
|---|---|
| Name | `ConsoleDev` (scripts assume this name) |
| OS | Windows 11 (64-bit) |
| RAM / CPUs | 8 GB / 4 vCPU or more |
| Disk | 80 GB dynamic (Steam/Epic test games need space) |
| Display | VBoxSVGA, 128 MB video memory, **3D acceleration on** (Slint's femtovg renderer uses OpenGL) |
| EFI + TPM 2.0 | On (needed by the Windows 11 installer) |
| USB | USB 3.0 (xHCI) controller, so a gamepad can be passed through |

Install Windows 11 and create a local user account.

## 2. Guest Additions + shared folder

1. In the VM window: **Devices → Insert Guest Additions CD image** → run `VBoxWindowsAdditions.exe` → reboot.
2. **Devices → Shared Folders → Settings → +**
   - Folder path: `/home/jonah/console-project`
   - Mount point: `Z:`
   - Auto-mount: yes, Make permanent: yes
3. Check that `Z:\touch.txt` opens inside the VM.

## 3. Toolchain inside the VM (Claude asks before each install)

| Tool | Why | How |
|---|---|---|
| Visual Studio 2022 Build Tools | MSVC linker + Windows SDK | Workload "Desktop development with C++" |
| rustup (stable, `x86_64-pc-windows-msvc`) | Rust compiler | https://rustup.rs |
| CMake ≥ 3.24 | Builds SDL3 from source (statically linked) | `winget install Kitware.CMake` |
| Git (optional) | Convenience | `winget install Git.Git` |

Build output goes to a directory local to the VM (`C:\cs-target`), not the shared folder.
Shared-folder I/O is slow, and Windows file locks on `.exe`/`.pdb` files behave badly over it:

```powershell
setx CARGO_TARGET_DIR C:\cs-target
```

## 4. Snapshots: not used

**This VM is run without snapshots (the user's decision; see ADR-010).** A *live* snapshot of this 8 GB
VM never converged. Its save file grew past 45 GB, and the VM had to be force-killed. If you ever want one,
shut the VM down first and take an **offline** snapshot, which is fast and small.

## 5. Build & run

The `Z:` share is **read-only**, which is deliberate: the VM can't modify host files. Builds therefore run on a
mirror at `C:\cs-src`, refreshed with `robocopy /MIR` before each build. The mirror keeps its own `Cargo.lock`.

From the host (`tools/vm.sh` wraps `VBoxManage guestcontrol`):

```bash
export CS_VM_PASSFILE=~/.cs-vm-pass          # file containing the VM user's password
tools/vm.sh --cargo build --release           # sync Z:\ -> C:\cs-src, then cargo build
tools/vm.sh --cargo test --workspace
tools/vm.sh --pull Cargo.lock                 # copy the lockfile back into the repo
```

Inside the VM by hand:

```powershell
robocopy Z:\ C:\cs-src /MIR /XD target .git /XF Cargo.lock
cd C:\cs-src
cargo build --release
C:\cs-target\release\Win2Console.exe
```

> VirtualBox 7 treats the arguments after `--` in `guestcontrol run` as argv[1..]. Do **not** repeat the
> program name there.

### Running and testing the UI from the host

```bash
tools/vm.sh --relaunch [release]          # restart Win2Console on the VM's desktop
tools/vm.sh --keys enter down right m     # drive it with keys (arrow/enter/esc/x/y/m/v/home/pgup/pgdn)
tools/vm.sh --type "Player One"           # type text
tools/vm.sh --shot m1-home                # screenshot -> docs/screenshots/m1-home.png
```

Notes from setting this up:
- Programs started by `guestcontrol` run on a hidden window station, so `--relaunch` asks the desktop's
  Explorer to start the app. Scheduled Tasks worked at first but later got stuck in "Queued".
- If a screenshot fails with `NS_ERROR_FAILURE`, Windows has turned the display off. Any key wakes it.

## 6. Screenshots (from the host)

```bash
VBoxManage controlvm ConsoleDev screenshotpng ~/console-project/docs/screenshots/m1-<screen>.png
```

Naming: `<milestone>-<screen>[-<state>].png`, e.g. `m1-profile-picker.png`, `m1-home-accent-green.png`.

## 7. Controller passthrough

One-time host setup: VirtualBox can only see host USB devices if your Linux user is in the `vboxusers`
group:

```bash
sudo usermod -aG vboxusers $USER   # then log out and back in (or reboot)
```

The VM has an xHCI (USB 3.0) controller and an auto-capture filter for the Xbox 360 controller
(`045e:028e`), so the pad attaches to the VM automatically whenever the VM is running:

```bash
VBoxManage usbfilter add 0 --target ConsoleDev --name "Xbox 360 Controller" --vendorid 045e --productid 028e
```

To use a different controller manually:

Plug the controller into the host, then in the VM window go to **Devices → USB → <controller>**. Xbox controllers
on the Xbox Wireless Adapter and DualSense/DualShock over USB both pass through this way. Bluetooth pads should be paired
to the host and then passed through as a USB adapter, or connected to the VM over USB.

## Tests

The pure-Rust crates (`cs-core` and the parser parts of `cs-library`) also build and test on the Linux host:

```bash
cargo test -p cs-core -p cs-library
```

The full workspace is tested in the VM with `cargo test --workspace`. Lint with `cargo clippy --workspace -- -D warnings`.
