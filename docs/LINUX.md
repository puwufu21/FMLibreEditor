# FMLibreEditor on Linux (Steam, Proton)

> **Status: experimental.** The Linux backend is new and has not yet been tested against a running game in every
> step below. Use an offline, single-player save, and back up the save before editing.

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

## Test, step by step

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
