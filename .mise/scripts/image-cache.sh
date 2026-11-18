#!/usr/bin/env bash

phase() {
  local name="$1"
  shift
  printf 'processing: %s\n' "$name"
  local start=${SECONDS}
  "$@"
  printf 'processing: %s - finished in: (%ds)\n' "$name" "$((SECONDS - start))"
}

hash_inputs() {
  python3 - "$@" <<'PY'
import hashlib
import os
import pathlib
import sys

digest = hashlib.sha256()
skip_dirs = {".git", "target", "backlog", ".cursor"}


def add_file(path: pathlib.Path, label: str) -> None:
    digest.update(label.encode())
    digest.update(b"\0")
    digest.update(path.read_bytes())
    digest.update(b"\0")


def add_tree(root: pathlib.Path) -> None:
    for dirpath, dirnames, filenames in os.walk(root, followlinks=False):
        dirnames[:] = sorted(name for name in dirnames if name not in skip_dirs)
        for name in sorted(filenames):
            if name.endswith(".md"):
                continue
            path = pathlib.Path(dirpath) / name
            if not path.is_file():
                continue
            add_file(path, path.relative_to(root).as_posix())


args = sys.argv[1:]
index = 0
while index < len(args):
    arg = args[index]
    if arg == "--tree":
        index += 1
        add_tree(pathlib.Path(args[index]).resolve())
    else:
        path = pathlib.Path(arg).resolve()
        if path.is_dir():
            add_tree(path)
        elif path.is_file():
            add_file(path, path.name)
        else:
            raise SystemExit(f"missing build input: {path}")
    index += 1
print(digest.hexdigest()[:16])
PY
}

reuse_or_build() {
  local tag="$1"
  shift
  if docker image inspect "$tag" >/dev/null 2>&1; then
    printf 'processing: docker image %s - reused\n' "$tag"
    return 0
  fi
  printf 'processing: docker image %s\n' "$tag"
  local start=${SECONDS}
  docker build -t "$tag" "$@"
  printf 'processing: docker image %s - finished in: (%ds)\n' "$tag" "$((SECONDS - start))"
}
