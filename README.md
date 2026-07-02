# sssh

A simple SSH client library and CLI built on [russh](https://github.com/warp-tech/russh).

## Features

- **Library-first** — reusable `Session` API with an `EventSink` trait for embedding
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
println!("stdout: {}", output.stdout);
println!("exit: {:?}", output.exit_code);
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
