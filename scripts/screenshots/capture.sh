#!/bin/bash
# Drives fx-gui on a virtual X server and crops one picture per feature into OUT.
# Runs only inside the throwaway container of `make screenshots`: it deletes and
# rebuilds $HOME for every picture.
set -euo pipefail

out=${1:?usage: capture.sh OUT}
here=$(cd "$(dirname "$0")" && pwd)
export DISPLAY=:99
pid=

pause() { sleep "${1:-0.6}"; }
key() { xdotool key --delay 80 "$@"; pause; }
text() { xdotool type --delay 60 "$1"; pause; }
move() { xdotool mousemove "$1" "$2"; pause 0.2; }
click() { move "$1" "$2"; xdotool click "${3:-1}"; pause; }
window() { xdotool search --name "$1" | tail -1; }
# palette NAME: runs a command through the command palette.
palette() { key ctrl+shift+p; text "$1"; key Return; }

# start DIR [SETTINGS_LINE...]: a fresh home, fresh settings, one fx window.
start() {
	local dir=$1
	shift
	stop
	find "$HOME" -mindepth 1 -delete
	"$here/fixtures.sh" "$HOME"
	cp "$here/state.toml" "$HOME/.config/fx/state.toml"
	printf '%s\n' 'restore_session = false' "$@" \
		>"$HOME/.config/fx/settings.toml"
	fx-gui "$dir" >/tmp/fx.log 2>&1 &
	pid=$!
	xdotool search --sync --name '^fx' >/dev/null
	pause 1.5
	xdotool windowfocus "$(window '^fx')"
	move 900 600
}

stop() {
	[ -n "$pid" ] || return 0
	kill "$pid" 2>/dev/null || true
	wait "$pid" 2>/dev/null || true
	pid=
}

# shot NAME [WIDTHxHEIGHT+X+Y]: the screen, cropped. Default is the main window.
shot() {
	pause
	import -window root -crop "${2:-1200x750+0+0}" +repage -strip "$out/$1.png"
}

# window_shot NAME TITLE: one native window, such as Settings.
window_shot() {
	pause
	import -window "$(window "$2")" -strip "$out/$1.png"
}

browsing() {
	start "$HOME"
	shot overview
	key ctrl+t
	click 80 209
	key ctrl+t
	click 80 253
	shot tabs 520x100+0+0
	start "$HOME/Projects/website/assets"
	shot breadcrumbs 700x34+0+63
	click 600 80
	shot path-edit 700x34+0+63
	start "$HOME"
	click 18 210
	pause
	shot sidebar 240x520+0+130
}

listing() {
	start "$HOME/Documents"
	shot columns 960x240+240+130
	click 336 113
	shot sort-menu 420x330+240+96
	start "$HOME/Projects/website"
	click 1090 80
	text "s"
	shot filter 1200x210+0+63
	start "$HOME/Documents"
	click 900 600
	text "inv"
	shot type-to-select 960x240+240+130
	start "$HOME" 'show_hidden = true'
	shot hidden-files 960x300+240+130
	start "$HOME/Documents" 'density = "comfortable"'
	shot density-comfortable 960x300+240+130
}

menus() {
	start "$HOME/Documents"
	click 26 14
	shot menu-file 320x300+0+0
	start "$HOME/Documents"
	click 73 14
	shot menu-view 320x230+0+0
	start "$HOME/Documents"
	click 330 238 3
	shot context-menu 520x500+240+130
	start "$HOME/Documents"
	key ctrl+shift+p
	shot command-palette 520x470+340+130
}

file_operations() {
	start "$HOME/Documents"
	key ctrl+shift+n
	text "Archive"
	shot new-folder 440x160+380+295
	start "$HOME/Documents"
	click 330 215
	key F2
	shot rename 440x160+380+295
	start "$HOME/Documents"
	click 330 215
	key Delete
	shot delete-confirm 440x120+380+318
	start "$HOME/Documents"
	click 330 215
	key alt+Return
	shot properties 440x270+380+240
	conflicts
	transfer
	drag_and_drop
}

conflicts() {
	start "$HOME/Documents"
	click 330 215
	palette Copy
	xdotool mousemove 330 192 click --repeat 2 --delay 80 1
	pause
	palette Paste
	shot conflict 440x200+380+278
}

transfer() {
	start "$HOME/Downloads"
	# Enough small files that the copy is still running when the picture is taken.
	mkdir "$HOME/Downloads/camera-roll"
	head -c 4096 /dev/urandom >/tmp/seed
	(cd "$HOME/Downloads/camera-roll" &&
		seq -f 'IMG_%06g.jpg' 100000 | xargs -n 500 sh -c 'tee "$@" </tmp/seed >/dev/null' sh)
	key F5
	pause 2
	click 330 169
	palette Copy
	click 80 253
	palette Paste
	shot transfer 1200x130+0+620
}

drag_and_drop() {
	start "$HOME/Documents"
	move 330 215
	xdotool mousedown 1
	move 340 205
	move 300 172
	shot drag-and-drop 520x240+0+130
	xdotool mouseup 1
}

searching() {
	start "$HOME"
	key ctrl+f
	text "re"
	pause 1
	shot search 1200x320+0+63
	start "$HOME/Projects"
	key ctrl+f
	text "*.rs"
	pause 1
	shot search-wildcard 1200x220+0+63
	start "$HOME"
	key ctrl+f
	text "unicorn"
	pause 1
	shot search-no-matches 1200x420+0+63
	unreadable_folders
	start /
	key ctrl+f
	text "demo"
	shot search-root-notice 440x190+380+300
}

unreadable_folders() {
	start "$HOME/Projects"
	mkdir "$HOME/Projects/private"
	chmod 000 "$HOME/Projects/private"
	key ctrl+f
	text "rs"
	pause 1
	click 345 741
	shot search-unreadable 1200x330+0+0
	chmod 700 "$HOME/Projects/private"
}

settings() {
	local names=(general appearance fonts listing behavior copy-and-move search keyboard)
	local i
	start "$HOME"
	key ctrl+comma
	xdotool search --sync --name '^Settings' >/dev/null
	pause 1
	for i in "${!names[@]}"; do
		xdotool mousemove --window "$(window '^Settings')" 90 $((52 + i * 28)) click 1
		window_shot "settings-${names[$i]}" '^Settings'
	done
}

appearance() {
	local theme
	for theme in nord-dark nord-light solarized-dark solarized-light dracula gruvbox-dark high-contrast; do
		start "$HOME/Documents" "theme = \"$theme\""
		click 330 215
		shot "theme-$theme" 640x300+0+96
	done
	start "$HOME" 'language = "de"'
	shot language 1200x340+0+0
	start "$HOME"
	key F1
	xdotool search --sync --name '^About' >/dev/null
	pause 1
	window_shot about '^About'
}

mkdir -p "$out"
Xvfb :99 -screen 0 1600x1000x24 >/tmp/x.log 2>&1 &
xpid=$!
trap 'stop; kill $xpid 2>/dev/null || true' EXIT
pause 1
for section in ${SECTIONS:-browsing listing menus file_operations searching settings appearance}; do
	"$section"
done
