#!/bin/sh
# List the immediate children of a directory for `Fs::read_dir`.
#
# `__DIR__` is replaced with a shell-quoted path by sssh before sending.
# `MODE=find` is followed by NUL-separated full paths (robust for names with
# spaces/newlines). If `find` is unavailable the caller falls back to `ls`.

dir=__DIR__

if [ ! -d "$dir" ]; then
  if [ -e "$dir" ] || [ -L "$dir" ]; then
    echo "ERROR=notdir"
  else
    echo "ERROR=notfound"
  fi
  exit 1
fi

if [ -L "$dir" ]; then
  # `find` does not follow a symlinked starting point; `ls` does, and its
  # relative names are joined to the link path by the caller (like std).
  printf 'MODE=ls\n'
  ls -1A "$dir" 2>/dev/null
  exit 0
fi

if command -v find >/dev/null 2>&1; then
  printf 'MODE=find\n'
  find "$dir" -mindepth 1 -maxdepth 1 -print0 2>/dev/null
else
  printf 'MODE=ls\n'
  ls -1A "$dir" 2>/dev/null
fi
