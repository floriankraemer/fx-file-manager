#!/usr/bin/env python3
"""Generates the copy benchmark datasets of issue #15 with a fixed seed.

Usage: bench/copy/gen.py OUT_DIR [--only D2,D3] [--scale 0.01] [--kernel-tarball linux-6.x.tar.xz]

D1  Linux kernel source tree (extracted from --kernel-tarball; skipped without it)
D2  500k files of 0-4 KiB in 2k directories
D3  node_modules-like tree: 200k files, deep nesting, symlinks and hardlinks
D4  5k files of 5 MiB
D5  13 x 512 MiB plus 1 x 20 GiB
D6  20 GiB sparse image with 2 GiB of data

--scale multiplies file counts and large sizes, for quick local runs.
"""

import argparse
import os
import random
import tarfile

MIB = 1024 * 1024
GIB = 1024 * MIB
SEED = 15


def payload(rng, size):
    # Random-ish but cheap: repeat a random 64 KiB block, so compression and dedup do not help.
    block = rng.randbytes(min(size, 64 * 1024))
    reps, rest = divmod(size, len(block)) if block else (0, 0)
    return block * reps + block[:rest]


def write(path, data):
    with open(path, "wb") as f:
        f.write(data)


def big_file(path, size, rng):
    with open(path, "wb") as f:
        chunk = rng.randbytes(8 * MIB)
        left = size
        while left > 0:
            n = min(left, len(chunk))
            f.write(chunk[:n])
            left -= n


def d1(out, args, rng):
    with tarfile.open(args.kernel_tarball) as tar:
        tar.extractall(out, filter="tar")


def d2(out, args, rng):
    dirs, files = max(1, int(2000 * args.scale)), max(1, int(500_000 * args.scale))
    for d in range(dirs):
        os.makedirs(os.path.join(out, f"dir{d:04}"), exist_ok=True)
    for i in range(files):
        write(os.path.join(out, f"dir{i % dirs:04}", f"f{i}"), rng.randbytes(rng.randint(0, 4096)))


def d3(out, args, rng):
    files = max(1, int(200_000 * args.scale))
    packages = max(1, files // 40)
    made = 0
    for p in range(packages):
        # Nested node_modules chains several levels deep, as npm produces them.
        depth = rng.randint(1, 6)
        parts = []
        for level in range(depth):
            parts += ["node_modules", f"pkg{p}-{level}"]
        root = os.path.join(out, *parts)
        for sub in ("lib", "dist", "src/utils"):
            os.makedirs(os.path.join(root, sub), exist_ok=True)
        for i in range(40):
            sub = ("lib", "dist", "src/utils")[i % 3]
            write(os.path.join(root, sub, f"m{i}.js"), payload(rng, rng.choice((90, 400, 1500, 9000))))
            made += 1
        os.symlink("lib/m0.js", os.path.join(root, "index.js"))
        os.link(os.path.join(root, "lib", "m3.js"), os.path.join(root, "dist", "m3.hard.js"))
        if made >= files:
            break


def d4(out, args, rng):
    for i in range(max(1, int(5000 * args.scale))):
        write(os.path.join(out, f"IMG_{i:05}.jpg"), payload(rng, 5 * MIB))


def d5(out, args, rng):
    for i in range(13):
        big_file(os.path.join(out, f"part{i:02}.bin"), int(512 * MIB * args.scale), rng)
    big_file(os.path.join(out, "huge.bin"), int(20 * GIB * args.scale), rng)


def d6(out, args, rng):
    size, data = int(20 * GIB * args.scale), int(2 * GIB * args.scale)
    with open(os.path.join(out, "disk.img"), "wb") as f:
        f.truncate(size)
        # 64 data islands spread over the image.
        island = data // 64
        for i in range(64):
            f.seek(i * (size // 64))
            f.write(payload(rng, island))


DATASETS = {"D1": d1, "D2": d2, "D3": d3, "D4": d4, "D5": d5, "D6": d6}


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("out")
    ap.add_argument("--only", default=",".join(DATASETS))
    ap.add_argument("--scale", type=float, default=1.0)
    ap.add_argument("--kernel-tarball")
    args = ap.parse_args()
    for name in args.only.split(","):
        if name == "D1" and not args.kernel_tarball:
            print("D1: skipped (pass --kernel-tarball)")
            continue
        out = os.path.join(args.out, name)
        if os.path.exists(out):
            print(f"{name}: exists, skipped")
            continue
        os.makedirs(out)
        DATASETS[name](out, args, random.Random(f"{SEED}-{name}"))
        print(f"{name}: {out}")


if __name__ == "__main__":
    main()
