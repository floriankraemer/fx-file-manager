# fx

Fast folder-listing core for a tabbed file explorer, plus the `fx` console app.

## Layout

`crates/fx-core` is the library: listing domain, storage detection and strategy, `list_dir` and `walk` use cases, and per-OS backends in `src/sys`.
Linux uses getdents64 and statx via rustix.
Windows uses GetFileInformationByHandleEx with FileFullDirectoryInfo.
macOS uses getattrlistbulk.
`std_fs` is the baseline and oracle.
`crates/fx-cli` is the binary `fx`.

## Storage strategies

- SSD: all cores.
- HDD: 1 stat thread, inode-ordered, and 2 walk threads.
- Network (SMB, NFS, 9p, FUSE, UNC, mapped drives): about 8 requests in flight and bigger buffers, because network listing is latency-bound.

## Usage

```
fx [PATH]  [--names] [--no-sort] [--backend native|std] [--storage ssd|hdd|network] [--quiet]
fx walk [PATH] [--print] [--backend native|std] [--storage ssd|hdd|network] [--quiet]
```

## Development

```
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo build --release
```

The oracle test (`crates/fx-core/tests/oracle.rs`) checks that the native backend equals the std backend.
