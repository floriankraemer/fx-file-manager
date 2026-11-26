# Features

A tour of the fx File Manager, one picture per feature.
All files, folders and drives shown are made up.

## Main window


![Main window with menu bar, tabs, toolbar, sidebar, file list and status line](images/features/overview.png)

## Navigation


### Tabs

Every tab has its own folder and history.

![Three tabs open](images/features/tabs.png)

### Breadcrumbs

Click a segment to jump to that folder.

![Breadcrumb bar](images/features/breadcrumbs.png)

### Typing a path

Click the empty part of the breadcrumb bar to type or paste a path.

![Path input](images/features/path-edit.png)

### Sidebar

Places, bookmarks and drives, each expandable as a folder tree.

![Sidebar with an expanded folder](images/features/sidebar.png)

## File list


### Columns

Name, extension, size, modification time and age.
Each optional column can be switched off in the settings.

![File list columns](images/features/columns.png)

### Sorting

Sort from the toolbar menu or by clicking a column header.

![Sort menu](images/features/sort-menu.png)

### Filter

Narrows the current folder as you type.

![Filtered file list](images/features/filter.png)

### Type to select

Start typing a name to jump to it, in the file list or the folder tree.

![Entry selected by typing](images/features/type-to-select.png)

### Hidden items

Shown on demand.

![Hidden folders in the list](images/features/hidden-files.png)

### Density

Compact (default, seen above) or comfortable rows.

![Comfortable row height](images/features/density-comfortable.png)

### Icons view

Each tab shows its folder as Details or as Icons: a grid of tiles with a preview or the file type icon, and the name below.
Switch with the View button in the command bar, the View menu, Ctrl+1 and Ctrl+2.
Ctrl+wheel changes the tile size; the arrow keys move through the grid.
Previews come from the operating system where it has them, so videos, PDFs and documents get one too: the shell thumbnail cache on Windows, QuickLook on macOS, and the shared `~/.cache/thumbnails` on Linux (with the tumbler service for non-images).
Images without a cached thumbnail are decoded by fx; on Linux the result is written back to the shared cache.

## Menus and commands


### Menu bar


![File menu](images/features/menu-file.png)

![View menu](images/features/menu-view.png)

### Context menu


![Context menu of a file](images/features/context-menu.png)

### Command palette

Ctrl+Shift+P lists every command by name.

![Command palette](images/features/command-palette.png)

## File operations


### New folder


![New folder dialog](images/features/new-folder.png)

### Rename

The name is preselected without its extension.

![Rename dialog](images/features/rename.png)

### Delete

Items go to the Trash by default.

![Delete confirmation](images/features/delete-confirm.png)

### Properties


![Properties dialog](images/features/properties.png)

### Drag and drop

Dropping moves; hold Ctrl (Option on macOS) to copy.

![Dragging a file onto a folder](images/features/drag-and-drop.png)

### Transfers

Copies and moves run in the background and can be paused or cancelled.
See [Copy engine](copy-engine.md).

![Transfer progress](images/features/transfer.png)

### Conflicts

Decide per file, or once for all remaining conflicts.

![Conflict dialog](images/features/conflict.png)

## Search

Ctrl+F searches below the current folder, without an index.
Results stream in while the walk runs.

![Search results with highlighted matches](images/features/search.png)

### Wildcards

`*` and `?` turn the query into a wildcard match.

![Wildcard search](images/features/search-wildcard.png)

### No matches


![Empty search result](images/features/search-no-matches.png)

### Unreadable folders

Folders that could not be read are listed with the reason.

![Unreadable folder list](images/features/search-unreadable.png)

### Searching a whole drive

A notice appears before the first search from a filesystem root.

![Root search notice](images/features/search-root-notice.png)

## Themes

Seven themes are built in, and you can [write your own](themes.md).

### Nord Dark


![Nord Dark theme](images/features/theme-nord-dark.png)

### Nord Light


![Nord Light theme](images/features/theme-nord-light.png)

### Solarized Dark


![Solarized Dark theme](images/features/theme-solarized-dark.png)

### Solarized Light


![Solarized Light theme](images/features/theme-solarized-light.png)

### Dracula


![Dracula theme](images/features/theme-dracula.png)

### Gruvbox Dark


![Gruvbox Dark theme](images/features/theme-gruvbox-dark.png)

### High Contrast


![High Contrast theme](images/features/theme-high-contrast.png)

## Languages

English, German, Spanish, French, Ukrainian and Simplified Chinese.
Here: German.

![German user interface](images/features/language.png)

## Settings

All options are described in [Settings](settings.md).

### General


![General settings](images/features/settings-general.png)

### Appearance


![Appearance settings](images/features/settings-appearance.png)

### Fonts


![Fonts settings](images/features/settings-fonts.png)

### Listing


![Listing settings](images/features/settings-listing.png)

### Behavior


![Behavior settings](images/features/settings-behavior.png)

### Copy and move


![Copy and move settings](images/features/settings-copy-and-move.png)

### Search


![Search settings](images/features/settings-search.png)

### Keyboard


![Keyboard settings](images/features/settings-keyboard.png)

## About


![About window](images/features/about.png)

## Console app

The `fx` binary lists, walks, searches, copies and moves from the command line, see [Console app](console.md).
