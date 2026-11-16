#!/bin/sh
# Print metadata for one path as KEY=value markers.
#
# `__PATH__` is replaced with a shell-quoted path by sssh before sending.
# `__FOLLOW_MODE__` is `yes` for metadata() (dereference symlinks) and `no`
# for symlink_metadata(); `__STAT_FLAG__` is the matching `stat -L` flag.
#
# GNU/BusyBox `stat -c` is tried first, then BSD/macOS `stat -f`. When neither
# works a coarse `TYPE=` marker is emitted so callers still learn the kind.

path=__PATH__
follow=__FOLLOW_MODE__
statflag='__STAT_FLAG__'

if [ "$follow" = yes ]; then
  if [ ! -e "$path" ]; then
    echo "NOTFOUND=1"
    exit 0
  fi
else
  if [ ! -e "$path" ] && [ ! -L "$path" ]; then
    echo "NOTFOUND=1"
    exit 0
  fi
fi

if [ -L "$path" ] && [ "$follow" = no ]; then
  echo "SYMLINK=yes"
  printf 'TARGET=%s\n' "$(readlink "$path" 2>/dev/null)"
fi

# GNU coreutils / BusyBox: %F type, %s size, %a mode, %u/%g ids, %Y mtime, %i inode.
stats=$(stat $statflag -c '%F|%s|%a|%u|%g|%Y|%i' "$path" 2>/dev/null) || stats=

if [ -z "$stats" ]; then
  # BSD / macOS: %HT type, %z size, %Lp mode, %u/%g ids, %m mtime, %i inode.
  stats=$(stat $statflag -f '%HT|%z|%Lp|%u|%g|%m|%i' "$path" 2>/dev/null) || stats=
fi

if [ -n "$stats" ]; then
  printf 'STAT=%s\n' "$stats"
else
  if [ -d "$path" ]; then
    echo "TYPE=dir"
  elif [ -f "$path" ]; then
    echo "TYPE=file"
  else
    echo "TYPE=other"
  fi
fi
