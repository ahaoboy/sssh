#!/bin/sh
# Print metadata for one path as KEY=value markers.
#
# `__PATH__` is replaced with a shell-quoted path by sssh before sending.
# `__FOLLOW_MODE__` is `yes` for metadata() (dereference symlinks) and `no`
# for symlink_metadata(); `__STAT_FLAG__` is the matching `stat -L` flag.
#
# GNU/BusyBox `stat -c` is tried first, then BSD/macOS `stat -f`. Hosts whose
# BusyBox build omits the `stat` applet (some routers) fall back to `ls`,
# `date` and `readlink`; a coarse `TYPE=` marker is the last resort.

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
  exit 0
fi

# Fallback for hosts without `stat`: derive the same fields from `ls -lnd`,
# `ls -id` and `date -r` (all common BusyBox applets).
lsflag=
if [ "$follow" = yes ]; then
  lsflag=-L
fi
mode_line=$(ls -lnd $lsflag "$path" 2>/dev/null) || mode_line=

if [ -n "$mode_line" ]; then
  set -- $mode_line
  perm=$1
  uid=$3
  gid=$4
  size=$5

  # GNU `ls` appends `+`/`.` for ACLs and SELinux; keep only the mode.
  perm=${perm%"${perm#??????????}"}

  case $perm in
    d*) kind='Directory' ;;
    l*) kind='symbolic link' ;;
    p*) kind='fifo' ;;
    s*) kind='socket' ;;
    c*) kind='character special file' ;;
    b*) kind='block special file' ;;
    -*) if [ -s "$path" ]; then kind='regular file'; else kind='regular empty file'; fi ;;
    *) kind='other' ;;
  esac

  # Size from `ls` matches `stat` for files, dirs and links; devices and
  # other special files report `major, minor` there, so pin them to 0.
  case $kind in
    'regular file' | 'regular empty file' | 'Directory' | 'symbolic link') ;;
    *) size=0 ;;
  esac

  # rwx triples -> octal, plus setuid/setgid/sticky (octal constants!).
  bits=${perm#?}
  mode=0
  while [ -n "$bits" ]; do
    triple=${bits%"${bits#???}"}
    value=0
    case $triple in r??) value=$((value | 4)) ;; esac
    case $triple in ?w?) value=$((value | 2)) ;; esac
    case $triple in ??[xst]) value=$((value | 1)) ;; esac
    mode=$((mode * 8 + value))
    bits=${bits#???}
  done
  case $perm in ???[sS]*) mode=$((mode | 04000)) ;; esac
  case $perm in ??????[sS]*) mode=$((mode | 02000)) ;; esac
  case $perm in ?????????[tT]) mode=$((mode | 01000)) ;; esac

  set -- $(ls -id $lsflag "$path" 2>/dev/null)
  inode=${1:-}
  mtime=$(date -r "$path" +%s 2>/dev/null)

  printf 'STAT=%s|%s|%o|%s|%s|%s|%s\n' \
    "$kind" "$size" "$mode" "$uid" "$gid" "$mtime" "$inode"
  exit 0
fi

if [ -d "$path" ]; then
  echo "TYPE=dir"
elif [ -f "$path" ]; then
  echo "TYPE=file"
else
  echo "TYPE=other"
fi
