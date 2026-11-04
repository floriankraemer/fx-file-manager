# Copy benchmarks

Go and no-go decisions for the Phase 6 experiments of issue #22 are taken on the real-hardware results of issue #21.
Until those exist, every experiment stays opt-in (`fx cp --experimental NAME`) and is not selected by `CopyPlan::for_pair`.

## Preliminary local A/B (not headline numbers)

Preliminary, WSL2 ext4 VHD, not headline.
These runs only check that each experiment works and roughly where it stands; they do not decide anything.

- Machine: WSL2 on Windows, 16 logical cores, ext4 on a virtual disk (not `/mnt/*`).
- Source and destination on the same ext4 filesystem, so the default chain is `copy_file_range` > `read-write`.
- Warm page cache (no root, so no cache drop), `hyperfine --warmup 1`, release build with `--features uring`.
- Datasets were generated in a temporary directory and deleted afterwards.

### Small files: io_uring (D2 scaled by 0.04: 20,000 files of 0-4 KiB, 79 MiB)

| Command | Mean wall time | Change |
|---|---:|---|
| default | 136.4 ms ± 8.7 | |
| `--experimental uring` | 355.2 ms ± 24.5 | 2.6x slower |
| `--verify` | 168.6 ms ± 13.6 | +23.6% |
| `--experimental uring --verify` | 381.9 ms ± 31.2 | 2.8x slower |

### Very large files: parallel ranged copy and O_DIRECT (2 files of 2 GiB, 4 GiB total)

| Command | Mean wall time | Change |
|---|---:|---|
| default | 14.53 s ± 0.58 | |
| `--experimental parallel` | 4.51 s ± 2.03 | 3.2x faster, high variance (min 3.4 s, max 8.1 s) |
| `--experimental o-direct` | 19.90 s ± 1.34 | 1.4x slower |
| `--verify` | 18.39 s ± 1.04 | +26.6% (the destination is still in the page cache) |

## Decisions

| Experiment | Local result | Status |
|---|---|---|
| io_uring small files | 2.6x slower here. The linked `READ` -> `WRITE` pair saves one syscall, but every worker pays for creating its own ring and the open and close calls stay synchronous. | Pending #21. Kept opt-in; the missing linked `OPENAT`/`CLOSE` and registered buffers are the likely reason. Drop it if #21 shows no gain on NVMe. |
| Parallel ranged copy | 3.2x faster here, but with high variance on a virtual disk, so the gain may come from write-back behaviour of the VHD. | Pending #21. |
| `O_DIRECT` for very large files | 1.4x slower here. The issue asks for it only if D5 shows write-back stalls. | Pending #21. Kept opt-in. |
| Raw Windows small-file path | Not measured, no Windows hardware here. It does not copy alternate data streams or security descriptors, so it does not meet the "equal fidelity" gate as written. | Pending #21. |
| xxh3 verification | Ships regardless of speed. Overhead here: +24% on small files and +27% on large files, with the destination read from the page cache. A cold destination read costs more. | Done. |
