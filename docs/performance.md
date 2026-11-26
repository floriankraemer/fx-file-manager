# Performance

How fx keeps listing, sorting and copying fast.
Copy internals are in [copy-engine.md](copy-engine.md); measurements are in [benchmarks/copy.md](benchmarks/copy.md).

- Linux reads directories with `getdents64` into one buffer per listing.
- Entry metadata is fetched with dirfd-relative `statx`, so no full paths are built.
- Names live in one byte arena and entries refer to them by offset.
- Stats fan out across threads only above 512 entries, so small folders stay serial.
- Rayon pools are cached per thread count instead of respawned per call.
- Large listings are sorted in parallel.
- Sorting and filtering operate on `u32` index vectors, not copies of entries.
- Background loads carry a generation counter, so stale results are dropped.
- The UI is fully decoupled from every other process, so it stays responsive.
  Listing, copying and moving run on background threads, and the UI only samples their state.
- Recursive search reuses the parallel walk (names and kinds only, no stat).
  Matching works on the name bytes and allocates nothing per entry.
  A folder's hit list is reserved once, on its first hit.
  Each hit path is one allocation of its exact size.
- Hidden filtering and the result cap run on the search worker, before hits reach the UI.
  The UI drains the hit channel for at most about 4 ms per frame and asks for another frame if events are left.
- Cancelling makes every worker stop before its next directory; measurements are in [development.md](development.md#search-benchmarks).
- Release builds use fat LTO, one codegen unit and `panic = "abort"`.

## File system reading strategies

Storage kind is detected per path and picks the concurrency strategy.

- SSD: one thread per core for stat and walk.
- HDD: one stat thread, stats sorted by inode to cut seeks, two walk threads.
- Network (SMB, NFS, 9p, FUSE, UNC, mapped drives): 8 requests in flight with 256 KiB buffers, since latency dominates.

## Picking up external changes

- Every tab's folder and every expanded tree folder is watched non-recursively with the OS change notifications: inotify on Linux, ReadDirectoryChangesW on Windows, FSEvents on macOS (one stream rebuild per watch-set change).
- Access events are filtered out in the kernel where the OS allows it, so fx reading a folder never triggers a re-list of it.
- Watches are added and removed on a worker thread, so a folder on a hung mount cannot stall the UI.
- A change re-lists the folder in the background without clearing the view; a burst of changes, such as a copy of thousands of files, costs at most one listing in flight plus one after it, and wakes the UI once per frame.
- On Windows, removable drives are not watched, since an open directory handle blocks ejecting them.
- File systems that deliver no notifications (many network mounts, Windows drives seen from WSL) fall back to a 2 s mtime check of the active folder and a re-list on window focus.

## Thumbnails

- The Icons view lays out only the visible rows.
  Each frame it asks for the thumbnails it lacks, visible ones first, then one screen ahead.
  That list replaces the queue, so entries scrolled out of view are never produced.
- OS caches come first: a hit is one small PNG (Linux) or a shell or QuickLook lookup.
  Thumbnails come in 128, 256 and 512 px buckets, the freedesktop sizes, so other applications' cache entries are hit directly and a larger entry serves a smaller request.
- JPEGs decode through the scaled IDCT at up to 1/8 size, so a 24 MP photo never builds its full image.
  Resizing uses SIMD (`fast_image_resize`).
  Files over 256 MiB or 100 megapixels are not decoded.
- Workers: one on HDD, USB and network storage, up to four on SSDs.
  Network storage gets no previews unless `thumbnails_on_network` is set.
- Files that yield no thumbnail are not retried until their mtime or size changes.
- Textures live in an LRU cache of about 256 MiB shared by all windows, and at most a dozen are uploaded per frame.

## Copy and move strategies

A pure planner picks workers, schedule and an ordered method chain for each pair of source and destination volume.
Unsupported methods are disabled for the pair on first failure, so a wrong guess costs speed, never correctness.

| Pair | Methods | Workers | Schedule |
|------|---------|---------|----------|
| Same volume, move | no-replace rename per item | 1 | - |
| Clone-capable (btrfs, XFS, bcachefs, ZFS, APFS, ReFS) | clone, then in-kernel copy | 2 x cores (4 on HDD/USB) | streaming |
| Linux, SSD | `copy_file_range` -> `sendfile` -> read/write | 2 x cores, 4-32 | streaming |
| Windows, SSD | `CopyFile2` | 2 x cores, 8-32 | streaming |
| Any HDD | in-kernel copy or `CopyFile2` | 1 | walk first, copy in disk order |
| USB | in-kernel copy or `CopyFile2` | 2 | streaming |
| Network | `copy_file_range` (server-side) or `CopyFile2` | 16 | streaming |

- Walk and copy run fused on a dedicated pool per transfer, so listing in other tabs never waits for a copy.
- Linux and macOS resolve every file relative to open directory handles, never by full path.
- Progress is a set of atomic counters sampled by the UI, not a message per file.
- Overwrites go through a temp file and a rename; a move deletes its source only after a flushed, error-free copy.
