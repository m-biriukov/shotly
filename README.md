# Screenshoot

A fast, lightweight screenshot tool for the macOS menu bar. Press a shortcut,
select an area (or click a window), draw on it with shapes, arrows and text,
then copy it to the clipboard or save it as a file.

Written in Rust. Everything stays on your Mac: the app never uses the network.

## Features

- **Capture** any area, or click a highlighted window to take exactly that
  window. Move and resize the selection with its handles. Works on several
  displays at once; images are saved at full Retina resolution.
- **Annotate** with a rectangle, ellipse, line, arrow, pen, highlighter, text
  and a pixelate tool for hiding sensitive information. Eight colors, three
  sizes, optional background box behind text.
- **Edit afterwards:** every shape stays an object you can select, move,
  resize, recolor or delete, and double-click text to edit it again. Unlimited
  undo and redo.
- **Output:** copy to the clipboard (PNG), or save as PNG or JPEG with your
  own folder and file-name pattern.
- **Feedback:** a notice slides in after copying or saving; click it to show
  the saved file in Finder. Its corner, size and distance from the screen edge
  are configurable.
- **Native feel:** a menu-bar app with a global shortcut, a native Settings
  window, launch at login, and keyboard shortcuts that work in any keyboard
  layout (Latin, Cyrillic and others).

## Requirements

- macOS 14 (Sonoma) or newer. It is developed and tested on Apple silicon;
  Intel Macs should build it too but are untested.
- Xcode Command Line Tools (the compiler and linker)
- Rust 1.88 or newer

Building it yourself takes a minute or two the first time; the prebuilt app
needs only macOS.

## Install

There are two ways: download the ready-made app (quickest), or build it
yourself.

### Option A: download the app

1. Download `Screenshoot-<version>-macos-arm64.zip` from the
   [latest release](https://github.com/m-biriukov/screenshoot/releases/latest)
   (Apple silicon Macs, macOS 14 or newer).
2. Unzip it and drag **Screenshoot.app** into `/Applications`.
3. The app is not signed with an Apple Developer ID, so macOS blocks the first
   launch with *"Apple could not verify…"*. Open it anyway with either:
   - **Control-click the app › Open › Open**, or
   - this command, which removes the download quarantine flag:

     ```sh
     xattr -dr com.apple.quarantine /Applications/Screenshoot.app
     ```
4. Continue with [First launch](#3-first-launch).

You can verify the download with the checksum published on the release page:
`shasum -a 256 Screenshoot-<version>-macos-arm64.zip`.

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
git clone https://github.com/m-biriukov/screenshoot.git
cd screenshoot
scripts/bundle.sh install
```

This builds a release version, wraps it in `Screenshoot.app`, signs it, and
copies it to `/Applications`. To build the app without installing it, run
`scripts/bundle.sh`; the result is `target/Screenshoot.app`.

### 3. First launch

(Both options.)

1. Open **Screenshoot** from `/Applications` (or Spotlight). It has no Dock
   icon; look for the viewfinder icon in the menu bar. If your menu bar is
   crowded, it may be hidden behind a menu-bar manager's collapse arrow.
2. Press the shortcut, **⌘⇧2** by default. macOS asks for **Screen Recording**
   permission (see [Permissions](#permissions-and-privacy)). Turn on
   *Screenshoot* in **System Settings › Privacy & Security › Screen
   Recording**, then quit Screenshoot from its menu-bar icon and open it again.
3. Press the shortcut again. Your screen freezes and dims: drag to select an
   area.

If another app already uses ⌘⇧2, Screenshoot says so in a notice; choose
another shortcut under **Settings…** in the menu-bar menu.

### Start at login

Menu-bar icon › **Launch at Login** (or the checkbox in Settings).

### Update

Download the newer release and replace the app in `/Applications`, or, if you
built it yourself, `git pull` and run `scripts/bundle.sh install` again. Both
replace the app; your settings are kept.

### Uninstall

Quit Screenshoot from its menu-bar icon, then:

```sh
rm -rf /Applications/Screenshoot.app
rm -rf ~/Library/Application\ Support/screenshoot
rm -f ~/Library/Logs/Screenshoot*.log
rm -f ~/Library/LaunchAgents/dev.screenshoot.agent.plist
```

Your screenshots in the save folder are not touched. You can also remove
*Screenshoot* from System Settings › Privacy & Security › Screen Recording.

### Other ways to run

```sh
cargo run --release              # run from the terminal as a menu-bar app
cargo run --release -- --capture # take one screenshot, then exit
```

When started from a terminal, macOS attributes the Screen Recording
permission to the terminal app instead, so the installed app is the easier
route.

## Using it

Press the shortcut, then **drag** to select an area or **click** a highlighted
window. A toolbar appears under the selection (hover any button for its name
and shortcut).

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
| Shift (while drawing) | Square, circle, 45° lines, straight highlighter |
| ⌘Z, ⇧⌘Z | Undo, redo |
| Delete | Delete the selected shape |
| Return or ⌘C | Copy to the clipboard |
| ⌘S | Save to the folder from Settings |
| Esc | Close a flyout, finish typing, or cancel the capture |

Shortcuts work in any keyboard layout: on a Russian or Ukrainian layout the
key in the R position is still R.

## Settings

Menu-bar icon › **Settings…**

| Setting | Notes |
| --- | --- |
| Capture shortcut | Click the button, then press the new combination |
| Save folder, File name | `{date}` and `{time}` are filled in; relative folders count from your home folder |
| Format | PNG, or JPEG (quality 90). The clipboard always gets PNG |
| Default color, Default size | Also remembered from your last capture |
| Show notice | Turn the "Copied / Saved" notice off |
| Notice position, size, distance from edge | **Preview** shows it without saving |
| Launch at login | |

Settings live in a plain file you can edit by hand (see
[Files](#files)).

## Permissions and privacy

Screenshoot needs one permission: **Screen Recording**, so macOS lets it see
what is on your displays. It asks the first time you take a screenshot, and
needs a restart after you grant it.

- Screenshots are kept in memory until you copy or save them, and are never
  sent anywhere. The app makes no network connections.
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
| Something else went wrong | Look at `~/Library/Logs/Screenshoot.log`; errors and crashes are recorded there |

### Keeping the permission across rebuilds

Create a signing certificate once: **Keychain Access › Certificate Assistant ›
Create a Certificate…**, name it **Screenshoot Dev**, identity type *Self
Signed Root*, certificate type *Code Signing*. `scripts/bundle.sh` uses it
automatically (or set `SCREENSHOOT_SIGN_IDENTITY` to another identity). Grant
the permission once more after the first signed build.

## Files

| What | Where |
| --- | --- |
| Settings | `~/Library/Application Support/screenshoot/config.toml` |
| Log (when not started from a terminal) | `~/Library/Logs/Screenshoot.log` |
| Launch at login | `~/Library/LaunchAgents/dev.screenshoot.agent.plist` |
| Screenshots | `~/Pictures/Screenshots` (configurable) |

## Development

```sh
cargo test
cargo fmt --check
cargo clippy --release --all-targets -- -D warnings
cargo audit        # needs: cargo install cargo-audit
```

CI runs all four on every push.

- `scripts/make_icon.sh` regenerates `assets/AppIcon.icns` from `examples/icon.rs`.
- `--features dev-hooks` enables developer switches: `SCREENSHOOT_PROFILE=1`
  (timing report), `SCREENSHOOT_TEST_CAPTURE=1` (auto-capture after launch),
  `SCREENSHOOT_TEST_TOAST=1` (sample notice) and `SCREENSHOOT_TEST_SETTINGS=1`
  (open Settings). Normal builds ignore them.

### How it is built

| Part | Files |
| --- | --- |
| App shell: menu bar, hotkey, notices | `app.rs`, `toast.rs`, `login.rs`, `macos.rs` |
| Capture (ScreenCaptureKit), window snapping | `capture.rs`, `snap.rs` |
| Capture session and window events | `overlay.rs` |
| Editing logic, no windowing (unit tested) | `editor.rs`, `editor/`, `shape.rs`, `geom.rs`, `keys.rs` |
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
- A selection stays within one display.

## License

[MIT](LICENSE)
