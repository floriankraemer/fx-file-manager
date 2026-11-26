#!/bin/sh
# Builds the fictional home folder shown in the documentation screenshots.
set -eu

home=${1:?usage: fixtures.sh HOME}

# file PATH SIZE DATE: a sparse file with a fixed size and modification time.
file() {
	mkdir -p "$(dirname "$home/$1")"
	truncate -s "$2" "$home/$1"
	touch -d "$3" "$home/$1"
}

file Documents/Reports/annual-report-2025.pdf 2400K "2026-02-12 09:14"
file Documents/Reports/budget-2026.xlsx 86K "2026-09-30 16:02"
file Documents/Reports/roadmap.pptx 5300K "2026-10-21 11:45"
file Documents/Notes/meeting-notes.md 4K "2026-11-02 10:05"
file Documents/Notes/ideas.txt 2K "2026-10-28 22:31"
file Documents/invoice-1042.pdf 182K "2026-11-05 08:20"
file Documents/contract.docx 64K "2026-08-17 13:37"
file Documents/Reports/contract.docx 58K "2026-05-04 10:20"
file Documents/travel-plan.md 7K "2026-11-11 19:12"
file Downloads/debian-13-netinst.iso 650M "2026-10-02 07:55"
file Downloads/fonts.zip 12M "2026-11-14 15:41"
file Pictures/Holiday/beach.jpg 3200K "2026-07-19 18:03"
file Pictures/Holiday/harbour.jpg 2900K "2026-07-20 12:44"
file Pictures/Holiday/sunset.png 4100K "2026-07-21 20:58"
file Pictures/avatar.png 48K "2026-03-03 10:00"
file Music/ambient.flac 31M "2026-01-09 21:15"
file Videos/demo.mp4 148M "2026-09-12 14:30"
file Projects/website/index.html 6K "2026-11-18 09:02"
file Projects/website/styles.css 11K "2026-11-18 09:40"
file Projects/website/app.js 23K "2026-11-19 17:26"
file Projects/website/README.md 3K "2026-11-01 12:00"
file Projects/website/package.json 1K "2026-10-15 08:08"
file Projects/website/assets/logo.svg 5K "2026-06-06 06:06"
file Projects/engine/src/main.rs 14K "2026-11-20 23:10"
file Projects/engine/src/lib.rs 9K "2026-11-17 14:55"
file Projects/engine/Cargo.toml 1K "2026-11-02 09:00"
file .config/fx/.keep 0 "2026-11-01 00:00"
mkdir -p "$home/Desktop"

# Directory times last, after their contents stopped changing them.
find "$home" -mindepth 1 -type d -exec touch -d "2026-11-10 12:00" {} +
