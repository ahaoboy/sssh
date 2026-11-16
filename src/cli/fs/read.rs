//! `sssh fs read` and `sssh fs ls`.

use tokio::io::AsyncWriteExt;

use crate::Fs;

use super::{LsArgs, ReadArgs, kind_char, mode_string};
use crate::Error;

pub(super) async fn read(fs: &Fs, args: &ReadArgs) -> Result<(), Error> {
    // `--text` validates UTF-8 before anything is written.
    if args.text {
        let text = fs.read_to_string(&args.path).await?;
        match &args.out {
            Some(path) => {
                tokio::fs::write(path, text.as_bytes()).await.map_err(|e| {
                    Error::Other(format!("failed to write '{}': {e}", path.display()))
                })?;
            }
            None => {
                let mut stdout = tokio::io::stdout();
                stdout.write_all(text.as_bytes()).await?;
                stdout.flush().await?;
            }
        }
        return Ok(());
    }

    match &args.out {
        Some(path) => {
            let file = tokio::fs::File::create(path)
                .await
                .map_err(|e| Error::Other(format!("failed to write '{}': {e}", path.display())))?;
            fs.download(&args.path, file).await?;
        }
        None => {
            fs.download(&args.path, tokio::io::stdout()).await?;
        }
    }
    Ok(())
}

pub(super) async fn ls(fs: &Fs, args: &LsArgs) -> Result<(), Error> {
    let mut entries = fs
        .read_dir(&args.path)
        .await?
        .collect::<Result<Vec<_>, _>>()?;
    entries.sort_by(|a, b| a.file_name().cmp(b.file_name()));

    for entry in entries {
        let name = if args.paths {
            entry.path().to_string_lossy().into_owned()
        } else {
            entry.file_name().to_string_lossy().into_owned()
        };

        if !args.long {
            println!("{name}");
            continue;
        }

        let meta = entry.metadata().await?;
        let kind = meta.file_type().kind();
        let mtime = meta
            .modified()
            .ok()
            .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_secs())
            .unwrap_or(0);
        println!(
            "{}{} {:>10} {:>12} {}",
            kind_char(kind),
            mode_string(meta.permissions().mode()),
            meta.len(),
            mtime,
            name,
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::super::format_bytes;

    #[test]
    fn long_size_uses_raw_bytes() {
        assert_eq!(format_bytes(1234), "1.2 KiB");
    }
}
