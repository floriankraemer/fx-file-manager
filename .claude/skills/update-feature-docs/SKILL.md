---
name: update-feature-docs
description: Review and update docs/features.md and its screenshots so they match the current app. Use after adding, changing or removing a user-visible feature, or when asked to update the feature page, retake screenshots, or check the feature documentation.
---

# Update the feature page

`docs/features.md` shows one picture per user-visible feature.
The pictures in `docs/images/features/` are generated, never edit or hand-capture them.

| File | Role |
| --- | --- |
| `scripts/screenshots/capture.sh` | One function per area; drives the GUI and calls `shot` or `window_shot` |
| `scripts/screenshots/fixtures.sh` | The made-up home folder |
| `scripts/screenshots/state.toml` | The made-up bookmarks |
| `docker/Dockerfile`, stages `screenshots*` | Fake user and drives |

## Privacy rule

Pictures come only from `make screenshots` or from a container without host mounts.
Never screenshot the app on the host, never bind-mount the checkout or a home folder into the capture container, and keep every name in the fixtures fictional.

## 1. Find what changed

Compare the page with the app, do not rely on memory:

- `git log --oneline $(git log -1 --format=%h -- docs/features.md)..HEAD` for changes since the page was last touched.
- `crates/fx-gui/src/command.rs` (commands), `action.rs` (intents) and `views/` (dialogs, panels, menus).
- `crates/fx-app/locales/en.toml` (every user-facing text) and `docs/settings.md`.
- `crates/fx-gui/themes/`, `crates/fx-app/locales/` and the settings categories, which the page lists one by one.

Write down which features are new, changed or removed before editing.

## 2. Edit script and page

- New feature: add the steps to the matching function in `capture.sh` (a new function also goes into the `SECTIONS` default list), add fixtures if needed, and add a section to `features.md`.
- Removed feature: delete its steps, fixtures and section.
- Crop to the relevant part of the app (`shot NAME WIDTHxHEIGHT+X+Y`), full window only for the overview.
- Picture names are kebab-case and name the feature.
- Each section has a heading, at most a few sentences (one per line), and a descriptive alt text.
- The main window is 1200x750 at 0,0 with no window manager.
- Prefer keyboard shortcuts or `palette "Command name"` over clicking coordinates.
  `xdotool key ctrl+c` and `ctrl+v` do not reach the file list, use `palette Copy` and `palette Paste`.
- `start` rejects duplicate keys: pass only the settings that differ from the defaults.

## 3. Iterate on one area

`make screenshots` takes several minutes, so work on single sections in a live container:

```sh
make build-linux linux-image
docker run -d --name fxshot --init -h demo-pc fx-linux-builder sleep 3600
docker cp dist/linux/fx-gui fxshot:/usr/local/bin/
docker cp scripts/screenshots fxshot:/shots
docker exec fxshot sh -c 'useradd -m demo && mkdir -p /mnt/backup /mnt/media /out && chown demo /out'
docker exec -u demo -e HOME=/home/demo -e SECTIONS="searching" fxshot /shots/capture.sh /out
docker cp fxshot:/out/. <scratch dir>
```

Repeat the two `docker cp` and `docker exec` steps after each edit (`docker exec -u root fxshot rm -rf /shots` first).
To find coordinates, take an uncropped `shot` and read them from the picture.
Remove the container with `docker rm -f fxshot` when done.

## 4. Regenerate and verify

1. Run `make screenshots`.
2. Look at every picture, for example as a contact sheet: `montage -label '%t' -geometry '600x400>+6+6' -tile 5x docs/images/features/*.png <scratch dir>/sheet.png`.
   A script that exits 0 proves nothing: check that the dialog or menu is open, the crop is not cut off, and the theme or language actually applied.
3. Check that page and pictures match, both commands must print nothing:

```sh
used=$(grep -oE 'features/[a-z0-9-]+\.png' docs/features.md | cut -d/ -f2 | sort -u)
have=$(ls docs/images/features | sort)
comm -23 <(echo "$used") <(echo "$have")
comm -13 <(echo "$used") <(echo "$have")
```

4. Restore pictures that differ only in timestamps or timings (`git checkout -- <file>`), so the diff shows real changes.
5. Report features added, changed and removed, plus any UI defect seen in the pictures.
