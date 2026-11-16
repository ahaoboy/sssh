//! Mutating `sssh fs` operations: write, mkdir, rm, mv, cp, chmod, ln, touch.

use std::io::Read;

use crate::Fs;

use super::{
    ChmodArgs, CpArgs, MkdirArgs, MvArgs, RmArgs, SymlinkArgs, TouchArgs, WriteArgs, parse_mode,
};
use crate::Error;

pub(super) async fn write(fs: &Fs, args: &WriteArgs) -> Result<(), Error> {
    let mode = args.mode.as_deref().map(parse_mode).transpose()?;

    if args.mkdirs {
        fs.create_parent_dirs(&args.path).await?;
    }

    // Stream local files instead of buffering them in memory.
    if let Some(path) = args.from.as_ref().filter(|_| !args.append) {
        let file = tokio::fs::File::open(path)
            .await
            .map_err(|e| Error::Other(format!("failed to read '{}': {e}", path.display())))?;
        fs.upload_with_mode(&args.path, file, mode).await?;
        return Ok(());
    }

    let data = read_input(args)?;

    if args.append {
        fs.append(&args.path, &data).await?;
        if let Some(mode) = mode {
            fs.set_mode(&args.path, mode).await?;
        }
    } else {
        fs.write_with_mode(&args.path, &data, mode).await?;
    }
    Ok(())
}

fn read_input(args: &WriteArgs) -> Result<Vec<u8>, Error> {
    if let Some(content) = &args.content {
        return Ok(content.as_bytes().to_vec());
    }
    if let Some(path) = &args.from {
        return std::fs::read(path)
            .map_err(|e| Error::Other(format!("failed to read '{}': {e}", path.display())));
    }

    let mut buffer = Vec::new();
    std::io::stdin().read_to_end(&mut buffer)?;
    Ok(buffer)
}

pub(super) async fn mkdir(fs: &Fs, args: &MkdirArgs) -> Result<(), Error> {
    if args.parents {
        fs.create_dir_all(&args.path).await?;
    } else {
        fs.create_dir(&args.path).await?;
    }
    Ok(())
}

pub(super) async fn rm(fs: &Fs, args: &RmArgs) -> Result<(), Error> {
    for path in &args.paths {
        let result = if args.recursive {
            fs.remove_dir_all(path).await
        } else if args.dir {
            fs.remove_dir(path).await
        } else {
            fs.remove_file(path).await
        };

        match result {
            Ok(()) => {}
            Err(e) if args.force && e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e.into()),
        }
    }
    Ok(())
}

pub(super) async fn mv(fs: &Fs, args: &MvArgs) -> Result<(), Error> {
    fs.rename(&args.from, &args.to).await?;
    Ok(())
}

pub(super) async fn cp(fs: &Fs, args: &CpArgs) -> Result<(), Error> {
    fs.copy(&args.from, &args.to).await?;
    Ok(())
}

pub(super) async fn chmod(fs: &Fs, args: &ChmodArgs) -> Result<(), Error> {
    let mode = parse_mode(&args.mode)?;
    for path in &args.paths {
        fs.set_mode(path, mode).await?;
    }
    Ok(())
}

pub(super) async fn symlink(fs: &Fs, args: &SymlinkArgs) -> Result<(), Error> {
    fs.symlink(&args.original, &args.link).await?;
    Ok(())
}

pub(super) async fn hard_link(fs: &Fs, args: &SymlinkArgs) -> Result<(), Error> {
    fs.hard_link(&args.original, &args.link).await?;
    Ok(())
}

pub(super) async fn touch(fs: &Fs, args: &TouchArgs) -> Result<(), Error> {
    for path in &args.paths {
        fs.touch(path).await?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_content_and_from_exclusively() {
        // Clap enforces the conflict; this only guards the literals path.
        let args = WriteArgs {
            target: "user@host".into(),
            path: "/tmp/x".into(),
            content: Some("hi".into()),
            from: None,
            mode: Some("600".into()),
            append: false,
            mkdirs: false,
        };
        assert_eq!(read_input(&args).unwrap(), b"hi");
        assert_eq!(parse_mode(args.mode.as_deref().unwrap()).unwrap(), 0o600);
    }
}
