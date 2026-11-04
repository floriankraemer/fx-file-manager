#!/usr/bin/env sh
# Runs the copy benchmarks of issue #15 with hyperfine: fx against the usual tools.
#
#   bench/copy/run.sh DATASETS_DIR DEST_DIR [D1 D2 ...]
#
# DEST_DIR decides the volume pair under test (same filesystem, second disk, HDD, USB, share).
# Each run starts from an empty destination; with root rights the page cache is dropped too (cold cache).
# Set WARM=1 to keep caches, RUNS=n to change the run count, FX=path to test another build.
# Results go to bench/copy/results/<dataset>.md and .json.
set -eu

src_root=${1:?datasets dir}
dst_root=${2:?destination dir}
shift 2
datasets=${*:-D1 D2 D3 D4 D5 D6}
runs=${RUNS:-5}
fx=${FX:-target/release/fx}
out=bench/copy/results
mkdir -p "$out" "$dst_root"

drop=""
if [ "${WARM:-0}" != 1 ]; then
    if [ "$(id -u)" = 0 ] && [ -w /proc/sys/vm/drop_caches ]; then
        drop="sync; echo 3 > /proc/sys/vm/drop_caches;"
    elif command -v purge >/dev/null 2>&1; then
        drop="sync; purge;"
    else
        echo "warning: cannot drop caches (not root); results are warm-cache" >&2
    fi
fi

for d in $datasets; do
    src="$src_root/$d"
    [ -d "$src" ] || { echo "skip $d: $src missing" >&2; continue; }
    dst="$dst_root/$d"
    set -- "$fx cp --quiet $src $dst"
    command -v cp >/dev/null && set -- "$@" "cp -a $src $dst"
    command -v xcp >/dev/null && set -- "$@" "xcp -r $src $dst"
    command -v fcp >/dev/null && set -- "$@" "fcp $src $dst"
    command -v rsync >/dev/null && set -- "$@" "rsync -a $src/ $dst/"
    hyperfine --runs "$runs" --export-markdown "$out/$d.md" --export-json "$out/$d.json" \
        --prepare "rm -rf $dst; $drop" --cleanup "rm -rf $dst" "$@"
done

# Same-volume move: must stay under 50 ms whatever the size.
for d in $datasets; do
    src="$src_root/$d"
    [ -d "$src" ] || continue
    back="if [ -e $src.moved ]; then $fx mv --quiet $src.moved $src; fi"
    hyperfine --runs "$runs" --export-markdown "$out/$d-mv.md" \
        --prepare "$back" --cleanup "$back" \
        "$fx mv --quiet $src $src.moved"
done
