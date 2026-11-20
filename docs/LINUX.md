# FMLibreEditor on Linux (Steam, Proton)

> **Status: experimental, tested.** Use an offline, single-player save, and back up the save before editing.

## Tested

| Item | Value |
|---|---|
| Date | 20 November 2026 |
| Game | FM27, Steam, version 27.1.0 (`game_plugin.dll` 27.1.0+2505117), Steam build 26620374 |
| System | Arch Linux, Hyprland, Proton-GE |
| Memory access | `kernel.yama.ptrace_scope=0` for the session |

Results:

- `fmlibre-probe info` found `fm.exe` and `game_plugin.dll`, storefront Steam, size of image `0x18878000`.
- `fmlibre-probe calibrate` chose the profile "FM27 Steam build 26620374" and found 124,420 players, 68,114 staff and
  31,869 clubs (100% of samples decoded).
- `fmlibre-probe health` passed all 12 checks.
- The app connected in a few seconds. The values agreed with FM. One attribute edit and its undo showed in FM, with no
  error.

Not tested yet: mass edit, freezing, contracts, transfers.

## Every time you use it

Do these steps in this order.

1. Back up your save. Copy the `.fm` file from
   `~/.local/share/Steam/steamapps/compatdata/<app id>/pfx/drive_c/users/steamuser/Documents/Sports Interactive/Football Manager 27/games/`.
2. Give the editor access to the game's memory. Do this one time after each restart of the computer:

   ```bash
   sudo sysctl kernel.yama.ptrace_scope=0
   ```

3. Start FM27 in Steam. Load your save. Wait for the main game screen.
4. Start the editor:

   ```bash
   ~/Tools/FMLibreEditor/target/release/FMLibreEditor
   ```

   If the window is blank or slow, start it with `WEBKIT_DISABLE_DMABUF_RENDERER=1` before the command.
5. Click **Connect**.
6. Edit. Click **Apply**. In FM, click **Continue** to see the change.
7. Save the game in FM. Edits stay in the save only after FM saves the game.

Also:

- If you load a different save in FM, click **Reload data** before you edit again.
- To cancel an edit, use **Changes** › **Undo** before you save in FM.
- If the editor says that this version of the game is new to it, FM was updated. Do not use **Edit anyway** until you
  have done "After an FM update" below.

## Update the editor

```bash
cd ~/Tools/FMLibreEditor && git pull
```

```bash
cd app && npm ci && npx tauri build --no-bundle && cd ..
```

```bash
cargo build --release -p fmlibre-probe
```

## After an FM update

1. Update the editor (see above). Upstream may already have a profile for the new build.
2. Start FM27 and load your save.
3. Run the read-only checks:

   ```bash
   target/release/fmlibre-probe calibrate
   ```

   ```bash
   target/release/fmlibre-probe health
   ```

4. If `calibrate` names a build profile and every `health` check passes, edit as usual.
5. If there is no profile, or a check fails, use the editor for reading only. Wait for an update of the editor.

## How it works

On Linux, Football Manager 27 is the Windows game, run by Steam through Proton (Wine). FMLibreEditor runs as a native
Linux program beside it. It reads the game the same way as on Windows: the same `game_plugin.dll`, the same build
profiles (`profiles/fm27/steam-*.toml`) and the same layout. Only the access to the game's memory is different:

| | Windows | Linux |
|---|---|---|
| Find the game | ToolHelp snapshot | `/proc/*/comm` and `/proc/*/cmdline` (`fm.exe`) |
| Find `game_plugin.dll` | module snapshot | the `.dll` mapped from its start in `/proc/<pid>/maps`, sized by its PE headers |
| Read, write | `ReadProcessMemory`, `WriteProcessMemory` | `process_vm_readv`, `process_vm_writev` |
| Threads' stacks | each thread's TEB | the TEBs Wine keeps on the heap, found by what each says of itself |
| New memory in the game | `VirtualAllocEx` | **not available** (see below) |

The Steam build is read from `steamapps/appmanifest_*.acf`, as on Windows, so the profile for your build is chosen
in the same way.

## Build

Arch Linux:

```bash
sudo pacman -S --needed base-devel rust nodejs npm webkit2gtk-4.1 gtk3 librsvg openssl
```

```bash
git clone -b linux-backend https://github.com/puwufu21/FMLibreEditor
```

```bash
cd FMLibreEditor/app && npm ci && npx tauri build --no-bundle
```

The program is `target/release/FMLibreEditor`. The command-line probe:

```bash
cd .. && cargo build --release -p fmlibre-probe
```

## Allow access to the game's memory

Linux lets one program read another program's memory only under the rules for debuggers (ptrace). With Arch's default
`kernel.yama.ptrace_scope = 1`, the editor is refused. FMLibreEditor does not change this setting itself. Do one of:

- For this session only (until the next restart):

  ```bash
  sudo sysctl kernel.yama.ptrace_scope=0
  ```

- Or give the probe the capability (do this again after each build):

  ```bash
  sudo setcap cap_sys_ptrace+ep target/release/fmlibre-probe
  ```

  `setcap` on the GUI program also works for memory access, but it puts the program in "secure" mode. In that mode,
  some desktop features that WebKitGTK uses (D-Bus portals, for example) can stop working. For the GUI, use the
  `sysctl` setting first.

If access is refused, the editor says so, with `errno 1` (`EPERM`).

## First test, step by step

1. Back up your save.
2. Start FM27 in Steam and load the save. Wait for the main game screen.
3. Check the probe first. It only reads:

   ```bash
   target/release/fmlibre-probe info
   ```

   ```bash
   target/release/fmlibre-probe calibrate
   ```

   ```bash
   target/release/fmlibre-probe health
   ```

   `info` must show `game_plugin.dll`, the storefront `Steam` and the Steam build (for example `26620374`).
   `calibrate` must find players, staff, clubs and the date. `health` must pass its checks.
4. Start the editor: `target/release/FMLibreEditor`. Click **Connect**.
5. Open one player. Compare the values with the player's screen in FM.
6. Change one value (for example, one attribute). Click **Apply**. In FM, click **Continue**, then look at the player.
7. Undo the change in **Changes**.

If a step fails, run `target/release/fmlibre-probe diagnostics` and keep its output. It contains no names or paths.

## Limits on Linux

- **No new memory in the game.** On Windows, the editor can allocate memory in the game (`VirtualAllocEx`). Linux has
  no way to do this from outside the process. The features that need it fail without a change to the game:
  - a stadium name longer than the old one
  - adding to a full list: some transfers, signings, staff moves and new relations between people

  All other edits work in the same way as on Windows.
- **No self-update.** "Check for updates" opens the release page. Upstream releases have no Linux build. Pull and
  build this branch again.
- **Graphics packs** are read from the Proton prefix of the game
  (`steamapps/compatdata/<app id>/pfx/drive_c/users/steamuser/Documents/Sports Interactive/Football Manager 27/graphics`).
  Set `FMLIBRE_GRAPHICS` to use a different folder.
