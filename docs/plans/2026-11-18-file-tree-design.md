# Remote File Tree Design

## Goal

Replace the flat remote file table with a lazy Windows directory tree that supports
context actions, drag-and-drop upload, drag-and-drop move and local-only icons.

## Backend

- `fs_list` with an empty path returns all Windows drive roots.
- `fs_list` lazily lists one directory at a time.
- `fs_move` moves a file/directory with `std::fs::rename`; cross-volume moves fall
  back to recursive copy + delete. Existing targets are rejected, and a directory
  cannot be moved into itself.
- `fs_download` accepts a directory: the Agent compresses it to a temporary zip via
  `Compress-Archive`, uploads the archive through the existing download request,
  then removes the temporary file.
- The Center stores `fs_move` as an assist file request and exposes
  `POST /api/v1/assist/sessions/{id}/fs/move`.

## Frontend

- `FileTree.tsx` loads drives on mount and expands directories on demand.
- Files and directories have inline SVG icons. No CDN, external font or icon
  package is used.
- A custom fixed-position context menu provides:
  - file: download, copy path, refresh;
  - directory: zip-and-download, upload to this directory, copy path, refresh.
- External file drops use the existing WebSocket chunk upload first, HTTP fallback
  second, targeting the directory under the pointer.
- Tree nodes are HTML5-draggable. Dropping a node on a directory calls `fs_move`.
- After upload/move the affected source and target directories refresh.

## Scope decisions

- No delete/rename in this iteration; not requested.
- Move conflicts do not overwrite.
- Tree loading is lazy; the whole disk is never scanned.
