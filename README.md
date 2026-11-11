# Shotly

A fast, lightweight screenshot and screen-recording tool for the macOS menu bar. Press a shortcut,
select an area (or click a window), draw on it with shapes, arrows and text,
then copy it to the clipboard or save it as a file. A second shortcut records
the same area as an MP4 video, with the mouse pointer and clicks highlighted.

Written in Rust. Everything stays on your Mac: the app never uses the network.

## Features

- **Capture** any area, or click a highlighted window to take exactly that
  window. Move and resize the selection with its handles. Works on several
  displays at once; images are saved at full Retina resolution.
- **Presets:** a strip at the top of the picker switches between **Full
  screen** (the display under the pointer), **Window** (click one) and
  **Custom** (drag an area with the mouse). The same strip works for
  screenshots and for video. The last one you used is remembered.
- **Annotate** with a rectangle, ellipse, line, arrow, pen, highlighter, text
  and a pixelate tool for hiding sensitive information. Eight colors, three
  sizes, optional background box behind text.
- **Edit afterwards:** every shape stays an object you can select, move,
  resize, recolor or delete, and double-click text to edit it again. Unlimited
  undo and redo.
- **Output:** copy to the clipboard (PNG), or save as PNG or JPEG with your
  own folder and file-name pattern.
- **Record video:** record an area or a window as an H.264 MP4, optionally
  with the microphone. A halo follows the mouse pointer and every click shows
  a ripple (left and right buttons in different colors), so viewers can follow
  what you do. A 3-2-1 countdown, a red menu-bar icon with the elapsed time
  and a frame around the area show that recording is on. A recorded window is
  followed when you move it.
- **Feedback:** a notice slides in after copying or saving; click it to show
  the saved file in Finder. Its corner, size and distance from the screen edge
  are configurable.
- **Native feel:** a menu-bar app with a global shortcut, a native Settings
  window, launch at login, and keyboard shortcuts that work in any keyboard
  layout (Latin, Cyrillic and others).

## Requirements

- macOS 15 (Sequoia) or newer. It is developed and tested on Apple silicon;
  Intel Macs should build it too but are untested.
- Xcode Command Line Tools (the compiler and linker)
- Rust 1.88 or newer

Building it yourself takes a minute or two the first time; the prebuilt app
needs only macOS.

## Install

There are two ways: download the ready-made app (quickest), or build it
yourself.

### Option A: download the app

1. Download `Shotly-<version>-macos-arm64.zip` from the
   [latest release](https://github.com/m-biriukov/shotly/releases/latest)
   (Apple silicon Macs, macOS 15 or newer).
2. Unzip it and drag **Shotly.app** into `/Applications`.
3. The app is not signed with an Apple Developer ID, so macOS blocks the first
   launch with *"Apple could not verify…"*. Allow it with this command, which
   removes the download quarantine flag:

   ```sh
   /usr/bin/xattr -dr com.apple.quarantine /Applications/Shotly.app
   ```

   (Use the full `/usr/bin/xattr` path: some setups, for example Homebrew
   tools, put a different `xattr` first on your `PATH`, and it lacks `-r`.)

   Or without the terminal: try to open the app once, then go to **System
   Settings › Privacy & Security**, scroll to *Security* and click **Open
   Anyway**.
4. Continue with [First launch](#3-first-launch).

You can verify the download with the checksum published on the release page:
`shasum -a 256 Shotly-<version>-macos-arm64.zip`.

### Option B: build it yourself

#### 1. Install the prerequisites

```sh
xcode-select --install
```

Then install Rust from <https://rustup.rs> (or run `rustup update` if you
already have it), and open a new terminal so `cargo` is on your `PATH`.
Check the versions:

```sh
cargo --version    # 1.88 or newer
```

#### 2. Get the source and build the app

```sh
git clone https://github.com/m-biriukov/shotly.git
cd shotly
scripts/bundle.sh install
```

This builds a release version, wraps it in `Shotly.app`, signs it, and
copies it to `/Applications`. To build the app without installing it, run
`scripts/bundle.sh`; the result is `target/Shotly.app`.

### 3. First launch

(Both options.)

1. Open **Shotly** from `/Applications` (or Spotlight). It has no Dock
   icon; look for the viewfinder icon in the menu bar. If your menu bar is
   crowded, it may be hidden behind a menu-bar manager's collapse arrow.
2. Press the shortcut, **⌘⇧2** by default. macOS asks for **Screen Recording**
   permission (see [Permissions](#permissions-and-privacy)). Turn on
   *Shotly* in **System Settings › Privacy & Security › Screen
   Recording**, then quit Shotly from its menu-bar icon and open it again.
3. Press the shortcut again. Your screen freezes and dims: drag to select an
   area.

If another app already uses ⌘⇧2, Shotly says so in a notice; choose
another shortcut under **Settings…** in the menu-bar menu.

### Start at login

Menu-bar icon › **Launch at Login** (or the checkbox in Settings).

### Update

Download the newer release and replace the app in `/Applications`, or, if you
built it yourself, `git pull` and run `scripts/bundle.sh install` again. Both
replace the app; your settings are kept.

### Upgrading from Screenshoot

Shotly is the new name of Screenshoot (version 0.2.0). Install it as usual;
on first start it copies your settings from the old folder and, if "Launch at
login" was on, moves it to the new app. Then:

- Delete `/Applications/Screenshoot.app` (`scripts/bundle.sh install` does it
  for you).
- macOS treats Shotly as a new app, so grant **Screen Recording** to it again
  in System Settings › Privacy & Security, and remove the old entry.
- The old settings folder `~/Library/Application Support/screenshoot` and
  log `~/Library/Logs/Screenshoot.log` are left in place; delete them when
  you like.

### Uninstall

Quit Shotly from its menu-bar icon, then:

```sh
rm -rf /Applications/Shotly.app
rm -rf ~/Library/Application\ Support/shotly
rm -f ~/Library/Logs/Shotly*.log
rm -f ~/Library/LaunchAgents/dev.shotly.agent.plist
```

Your screenshots in the save folder are not touched. You can also remove
*Shotly* from System Settings › Privacy & Security › Screen Recording.

### Other ways to run

```sh
cargo run --release              # run from the terminal as a menu-bar app
cargo run --release -- --capture # take one screenshot, then exit
```

When started from a terminal, macOS attributes the Screen Recording
permission to the terminal app instead, so the installed app is the easier
route.

## Using it

Press the shortcut. A strip at the top of the screen offers three presets
(or press **F**, **W**, **C**):

| Preset | What it does |
| --- | --- |
| Full screen | Selects the whole display under the pointer at once; click the strip on another display to use that one |
| Window | Hover to highlight a window, **click** it to select it (dragging does nothing here) |
| Custom | **Drag** an area with the mouse |

The size label above the selection shows its size in screen pixels (a Retina
display has two pixels per point). A screenshot is saved at exactly that size;
a video is too, unless Settings › Recording › Resolution is *Standard*, which
records one pixel per point. You can still move and resize any selection with
its handles.

A toolbar appears under the selection (hover any button for its name and
shortcut).

| Toolbar | What it does |
| --- | --- |
| Select | Move the selection; select, move and resize shapes |
| Shapes ▾ | Rectangle, ellipse, line |
| Arrow | Arrow |
| Draw ▾ | Pen, highlighter |
| Text | Click and type; double-click a text to edit it again |
| Blur | Pixelate a rectangle |
| Color ▾, Size ▾ | Palette, stroke/text/block size, text background box |
| Undo, Redo | Step back and forward |
| Copy, Save, Close | Finish, or cancel |

| Key | Action |
| --- | --- |
| V R O L A P H T B | Select, Rectangle, Ellipse, Line, Arrow, Pen, Highlighter, Text, Blur |
| F W C | Preset: Full screen, Window, Custom |
| Shift (while drawing) | Square, circle, 45° lines, straight highlighter |
| ⌘Z, ⇧⌘Z | Undo, redo |
| Delete | Delete the selected shape |
| Return or ⌘C | Copy to the clipboard |
| ⌘S | Save to the folder from Settings |
| Esc | Close a flyout, finish typing, or cancel the capture |

Shortcuts work in any keyboard layout: on a Russian or Ukrainian layout the
key in the R position is still R.

## Recording video

Press the record shortcut, **⌘⇧1** by default (or menu-bar icon › **Record
Screen**). The screen freezes as for a screenshot: drag an area or click a
window (the same presets as for screenshots), adjust it with the handles, then
click the red **Record** button or press Return. Esc cancels.

After the countdown the recording starts and the menu-bar icon turns red and
shows the elapsed time. Stop it with the same shortcut, or menu-bar icon ›
**Stop Recording**. Quitting the app while recording stops and saves the file
first. The video is saved as `.mp4` in the save folder, with the file-name
pattern from Settings; the notice that follows works like the one for
screenshots (click it to show the file in Finder).

- A recorded window is followed when you move it, as long as you picked it
  with a click and did not change the selection (a window that reaches past
  the edge of the display is recorded as a plain area). The recording
  captures that part of the screen, so a window that passes in front of it
  shows up in the video. Resizing the window does not change the video size.
- If you drag the area by hand, the recording keeps that area of the screen.
- While recording, the screenshot shortcut only shows a notice (its overlay
  would end up in the video), and **Settings…** is unavailable.
- The area stays within one display.
- Sound: only the microphone, and only if you turn it on in Settings. If
  macOS denies the Microphone permission, the video is recorded without
  sound and the notice says so.

## Settings

Menu-bar icon › **Settings…**. Three tabs:

**General**

| Setting | Notes |
| --- | --- |
| Capture shortcut | Click the button, then press the new combination |
| Save folder, File name | `{date}` and `{time}` are filled in; relative folders count from your home folder |
| Format | PNG, or JPEG (quality 90). The clipboard always gets PNG |
| Default color, Default size | Also remembered from your last capture, as is the last preset |
| Launch at login | |

**Notice**

| Setting | Notes |
| --- | --- |
| Show notice | Turn the "Copied / Saved" notice off |
| Position, size, distance from edge | **Preview** shows it without saving |

**Recording**

| Setting | Notes |
| --- | --- |
| Record shortcut | Starts and stops a recording (default ⌘⇧1) |
| Frame rate | 30 or 60 fps |
| Resolution | Retina (the display's full pixels) or Standard (one pixel per point, about a quarter of the size) |
| Record microphone | Off by default |
| Count down 3-2-1, Show a frame around the area | On by default |
| Highlight the mouse pointer, color, size | A halo around the pointer |
| Show mouse clicks, left and right click color | A ripple at every click |

Settings live in a plain file you can edit by hand (see
[Files](#files)).

## Permissions and privacy

Shotly needs one permission: **Screen Recording**, so macOS lets it see
what is on your displays. It asks the first time you take a screenshot, and
needs a restart after you grant it. Video recording uses the same permission.
If you turn on **Record microphone**, macOS also asks for **Microphone**
access. The pointer halo and click ripples work without any extra permission;
the app does not watch the keyboard.

- Screenshots are kept in memory until you copy or save them, and are never
  sent anywhere. Videos are written straight to the save folder. The app makes no network connections.
- Saved files are readable only by you (mode `600`).
- Anything you copy sits on the clipboard like any other copied image, so
  other apps (and Universal Clipboard) can read it.
- Pixelate uses coarse blocks so that text underneath is hard to recover, but
  the safest way to protect a password or key is to keep it out of the
  selection.

## Troubleshooting

| Problem | Fix |
| --- | --- |
| Flat grey screen, or a notice "Screen Recording permission needed" | Grant the permission (step 3 above), then restart the app |
| macOS asks for the permission again after you rebuilt the app | Builds are signed ad hoc, so macOS sees a new app each time; see [Keeping the permission across rebuilds](#keeping-the-permission-across-rebuilds) |
| "Shortcut … is used by another app" | Pick another one in Settings… |
| No menu-bar icon | Check behind your menu-bar manager's collapse arrow, or look at the log (below) |
| Something else went wrong | Look at `~/Library/Logs/Shotly.log`; errors and crashes are recorded there |

### Keeping the permission across rebuilds

Create a signing certificate once: **Keychain Access › Certificate Assistant ›
Create a Certificate…**, name it **Shotly Dev**, identity type *Self
Signed Root*, certificate type *Code Signing*. `scripts/bundle.sh` uses it
automatically (or set `SHOTLY_SIGN_IDENTITY` to another identity). Grant
the permission once more after the first signed build.

## Files

| What | Where |
| --- | --- |
| Settings | `~/Library/Application Support/shotly/config.toml` |
| Log (when not started from a terminal) | `~/Library/Logs/Shotly.log` |
| Launch at login | `~/Library/LaunchAgents/dev.shotly.agent.plist` |
| Screenshots and videos | `~/Pictures/Screenshots` (configurable) |

## Development

```sh
cargo test
cargo fmt --check
cargo clippy --release --all-targets -- -D warnings
cargo audit        # needs: cargo install cargo-audit
```

CI runs all four on every push.

- `scripts/make_icon.sh` regenerates `assets/AppIcon.icns` from `assets/icon-source.png`
  (needs ImageMagick: `brew install imagemagick`).
- `scripts/make_menubar_icon.sh` regenerates the menu-bar icon
  `assets/menubar-icon.png` from `assets/menubar-source.png`.
- `--features dev-hooks` enables developer switches: `SHOTLY_PROFILE=1`
  (timing report), `SHOTLY_TEST_CAPTURE=1` (auto-capture after launch),
  `SHOTLY_TEST_TOAST=1` (sample notice), `SHOTLY_TEST_SETTINGS=1`
  (open Settings) and `SHOTLY_TEST_RECORD=1` (open the recording area
  picker after launch; picking an area then starts a real recording).
  Normal builds ignore them.

### How it is built

| Part | Files |
| --- | --- |
| App shell: menu bar, notices | `app.rs`, `toast.rs`, `tray.rs`, `login.rs`, `macos.rs` |
| Global shortcuts | `app/hotkeys.rs` |
| Capture (ScreenCaptureKit), window snapping | `capture.rs`, `snap.rs` |
| Video recording: state machine in the app, stream, halo, ripples, countdown | `app/recording.rs`, `record.rs`, `highlight.rs` |
| Capture session and window events | `overlay.rs` |
| Editing logic, no windowing (unit tested), presets | `editor.rs`, `editor/` (incl. `presets.rs`), `shape.rs`, `geom.rs`, `keys.rs` |
| Drawing: shapes, text, export | `paint.rs`, `text.rs`, `output.rs` |
| Overlay display (Core Animation layers) | `layers.rs`, `render.rs`, `toolbar.rs` |
| Settings and config | `settings.rs`, `config.rs` |
| Look | `style.rs`, `theme.rs` |

Drawing uses [tiny-skia](https://github.com/RazrFalcon/tiny-skia) for both the
on-screen overlay and the exported image, so what you see is what you save.

## Known limitations

- macOS only.
- The overlay appears about 75–80 ms after the shortcut; ScreenCaptureKit
  alone needs about 60 ms of that.
- Text supports Latin and Cyrillic and other simple scripts; there is no
  emoji, no right-to-left or complex-script shaping, and no kerning.
- A selection, and so a recording, stays within one display.
- A keystroke overlay for recordings is not available yet.
- Recordings have no pause, and no system sound.

## License

[MIT](LICENSE)
