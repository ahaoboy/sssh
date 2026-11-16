# sssh

A simple SSH client library and CLI built on [russh](https://github.com/warp-tech/russh).

## Features

- **Library-first** — reusable `Session` API with an `EventSink` trait for embedding
- **Remote filesystem API** — a `std::fs`-like, `async` layer (`read`, `write`,
  `create_dir_all`, `metadata`, `read_dir`, `available_space`, …) over SSH
- **Streaming transfers** — `download`/`upload` copy through the SSH channel
  without buffering whole files in memory
- **`std::process`-shaped output** — `Output` with raw `stdout`/`stderr` bytes
  and an `ExitStatus` (`success()`, `code()`, `signal()`)
- **Three modes** — interactive shell, idle keep-alive, one-shot command execution
- **Configurable host-key verification** — plug in your own policy (TOFU, known_hosts)
- **Connection timeout** — prevent hangs on unreachable hosts
- **PTY resize** — terminal window changes are forwarded to the remote
- **Graceful disconnect** — proper SSH disconnect before closing TCP
- **CLI binary** — batteries-included `sssh` command (requires `cli` feature)

## Installation

```toml
[dependencies]
sssh = { git = "https://github.com/ahaoboy/sssh" }
```

For the CLI binary:

```bash
cargo install --git https://github.com/ahaoboy/sssh
```

## Usage

### Library — run a command

```rust
use sssh::{exec, Auth};

let output = exec("user@host", 22, Auth::password("pass"), "uname -a").await?;
println!("stdout: {}", output.stdout_lossy());
println!("exit: {:?}", output.code());
```

### Library — interactive shell

```rust
use sssh::{Config, EventSink, Session};

struct MySink;
impl EventSink for MySink {
    fn on_connected(&self) {}
    fn on_data(&self, data: String) { print!("{data}"); }
    fn on_closed(&self) {}
}

let config = Config {
    host: "example.com".into(),
    port: 22,
    username: "user".into(),
    password: Some("pass".into()),
    private_key: None,
    command: None,
    cols: 120,
    rows: 40,
    timeout_secs: Some(10),
    host_key_verifier: None, // defaults to accept-all (insecure)
};

let mut session = Session::connect(config, MySink).await?;
session.write("ls -la\n".as_bytes())?;
```

### Library — remote files (`std::fs`-like API)

`Client` is a non-interactive, reusable connection; `Client::fs` returns an
`Fs` handle with the familiar `std::fs` vocabulary. Every operation is
`async`, returns `std::io::Result`, and addresses the remote host. Paths are
POSIX: `~` points at the remote `$HOME` and relative paths resolve against it.

```rust
use sssh::{Auth, Client};

let client = Client::connect_target("user@host", 22, Auth::password("pass"), Some(10)).await?;
let fs = client.fs();

// Directories
fs.create_dir_all("~/.local/bin").await?;

// Files
fs.write("~/.local/bin/hello.txt", b"hello over ssh\n").await?;
fs.set_mode("~/.local/bin/hello.txt", 0o755).await?;
fs.append("~/.local/bin/hello.txt", b"more\n").await?;

assert_eq!(fs.read_to_string("~/.local/bin/hello.txt").await?.lines().count(), 2);
assert!(fs.exists("~/.local/bin/hello.txt").await?);

// Metadata / listing
let meta = fs.metadata("~/.local/bin/hello.txt").await?;
println!("{} bytes, mode {:o}", meta.len(), meta.permissions().mode());

for entry in fs.read_dir("~/.local/bin").await? {
    let entry = entry?;
    println!("{}", entry.path().display());
}

// Disk space (None when the host cannot report it)
if let Some(free) = fs.available_space("~").await? {
    println!("{free} bytes free");
}

// Large files: stream instead of buffering in memory
let mut local = tokio::fs::File::create("./backup.tar").await?;
let copied = fs.download("/var/backups/data.tar", &mut local).await?;
println!("{copied} bytes downloaded");

let upload = tokio::fs::File::open("./firmware.bin").await?;
fs.upload("/tmp/firmware.bin", upload).await?;
```

Implementations map to standard POSIX utilities (`cat`, `mkdir`, `rm`, `mv`,
`cp`, `ln`, `stat`, `df`), so they work on GNU/Linux, BusyBox (routers, Alpine),
and BSD/macOS. Writes are verified by size, so a full disk cannot silently
produce a truncated file.

| API | Maps to |
|-----|---------|
| `read`, `read_to_string`, `read_link`, `read_dir`, `try_exists`/`exists`, `is_file`, `is_dir` | `cat`, `readlink`, `find`/`ls`, `test` |
| `write`, `write_with_mode`, `append`, `create`, `touch` | `cat`/`touch` with size verification |
| `create_dir`, `create_dir_all`, `remove_file`, `remove_dir`, `remove_dir_all` | `mkdir`, `rm`, `rmdir` |
| `rename`, `copy`, `canonicalize` | `mv`, `cp -p`, `readlink -f`/`pwd -P` |
| `symlink`, `hard_link`, `read_link` | `ln -s`, `ln`, `readlink` |
| `metadata`, `symlink_metadata`, `set_permissions`, `set_mode` | `stat`, `chmod` |
| `dir_status`, `disk_space`, `available_space`, `total_space` | `stat -f`/`df -Pk` |
| `download`, `upload`, `upload_with_mode` | `cat` over a streaming SSH channel |

The module is split by concern (`fs::{read, write, dir, link, stat, chmod,
space, stream}` plus `fs::types` and embedded probe scripts in
`src/fs/scripts/`), so new operations can be added without touching a monolith.

### CLI

```bash
# Interactive shell (default)
sssh user@host

# With a custom shell
sssh user@host -t /jffs/fish

# Execute a command non-interactively
sssh user@host -m cmd -c "ls -la"

# Idle keep-alive
sssh user@host -m idle

# With private key
sssh user@host -i ~/.ssh/id_rsa

# Verbose logging
sssh user@host -v

# Run cmd
sssh user@192.168.50.1 -p 8888 -i 'C:/Users/Admin/.ssh/router'  -t '/jffs/fish' -c "neofetch" -m cmd
```

### CLI — remote filesystem

`sssh fs` exposes the filesystem API from the command line. Connection flags
(`-p/--port`, `-i/--identity`, `--password`, `--timeout`) go before the
operation; the password may also come from `SSSH_PASSWORD` or the prompt.

```bash
# Read: raw bytes to stdout, UTF-8 check with --text, or save locally with -o
sssh fs read user@host ~/.bashrc
sssh fs read user@host /etc/motd --text
sssh fs read user@host /tmp/dump.bin -o ./dump.bin

# Write: stdin by default, or --content / --from FILE; --mkdirs, --mode, -a
printf 'hello\n' | sssh fs write user@host /tmp/hello.txt
sssh fs write user@host ~/.local/bin/tool --from ./tool --mkdirs --mode 755
sssh fs write user@host /tmp/log.txt --content "one line" --append

# Directories
sssh fs ls user@host /etc
sssh fs ls user@host /etc --long
sssh fs mkdir user@host /tmp/work --parents
sssh fs rm user@host /tmp/work --recursive
sssh fs rm user@host /tmp/a /tmp/b --force

# Inspect
sssh fs stat user@host /etc/hosts
sssh fs df user@host /
sssh fs exists user@host /etc/hosts   # exit 0 = exists, 1 = missing
sssh fs readlink user@host /usr/bin/sh
sssh fs canonicalize user@host /usr/bin/..

# Permissions and links
sssh fs chmod user@host 755 ~/tool
sssh fs mv user@host ~/old.txt ~/new.txt
sssh fs cp user@host ~/a.txt ~/b.txt
sssh fs symlink user@host target ~/link
sssh fs hardlink user@host ~/a.txt ~/b.txt
sssh fs touch user@host ~/empty
```

| Command | Library call |
|---------|--------------|
| `read`, `ls` | `Fs::download` / `read_to_string` / `read_dir` |
| `write` (`-a`, `--from`) | `Fs::upload_with_mode` / `write_with_mode` / `append` |
| `mkdir` (`-p`) | `Fs::create_dir` / `create_dir_all` |
| `rm` (`-r`, `-d`, `-f`) | `Fs::remove_file` / `remove_dir` / `remove_dir_all` |
| `mv`, `cp` | `Fs::rename` / `copy` |
| `symlink`, `hardlink`, `touch` | `Fs::symlink` / `hard_link` / `touch` |
| `stat`, `chmod` | `Fs::metadata` / `set_permissions` |
| `df`, `exists`, `readlink`, `canonicalize` | `Fs::dir_status` / `try_exists` / `read_link` / `canonicalize` |

### Host-key verification

Implement `HostKeyVerifier` for custom policies:

```rust
use sssh::HostKeyVerifier;
use russh::keys::ssh_key;

struct MyVerifier;
impl HostKeyVerifier for MyVerifier {
    fn verify(&self, host: &str, key: &ssh_key::PublicKey) -> Result<(), String> {
        // Check against known_hosts, prompt user, etc.
        Ok(())
    }
}

let config = Config {
    host_key_verifier: Some(std::sync::Arc::new(MyVerifier)),
    ..config
};
```

## License

MIT
