//! Read-only `sssh fs` operations: stat, df, exists, readlink, canonicalize.

use crate::Fs;

use super::{DfArgs, PathArg, StatArgs, format_bytes, print_metadata};
use crate::Error;

pub(super) async fn stat(fs: &Fs, args: &StatArgs) -> Result<(), Error> {
    let mut failed = false;
    for (index, path) in args.paths.iter().enumerate() {
        if index > 0 {
            println!();
        }
        match fs.metadata(path).await {
            Ok(meta) => print_metadata(path, &meta),
            Err(e) => {
                eprintln!("{path}: {e}");
                failed = true;
            }
        }
    }
    if failed {
        std::process::exit(1);
    }
    Ok(())
}

pub(super) async fn df(fs: &Fs, args: &DfArgs) -> Result<(), Error> {
    let status = fs.dir_status(&args.path).await?;
    println!("path: {}", status.path.display());
    println!("exists: {}", yes_no(status.exists));
    println!("writable: {}", yes_no(status.writable));
    println!("creatable: {}", yes_no(status.creatable));
    println!("probe: {}", status.space.probe_path.display());

    if status.space.known {
        println!(
            "total: {} ({})",
            status.space.total_bytes,
            format_bytes(status.space.total_bytes)
        );
        println!(
            "available: {} ({})",
            status.space.available_bytes,
            format_bytes(status.space.available_bytes)
        );
    } else {
        println!("total: unknown");
        println!("available: unknown");
    }

    if let Some(error) = &status.error {
        println!("error: {error}");
    }
    Ok(())
}

pub(super) async fn exists(fs: &Fs, args: &PathArg) -> Result<(), Error> {
    if fs.exists(&args.path).await? {
        Ok(())
    } else {
        // `test -e` semantics: silent, with a non-zero exit status.
        std::process::exit(1);
    }
}

pub(super) async fn readlink(fs: &Fs, args: &PathArg) -> Result<(), Error> {
    let target = fs.read_link(&args.path).await?;
    println!("{}", target.display());
    Ok(())
}

pub(super) async fn canonicalize(fs: &Fs, args: &PathArg) -> Result<(), Error> {
    let path = fs.canonicalize(&args.path).await?;
    println!("{}", path.display());
    Ok(())
}

fn yes_no(value: bool) -> &'static str {
    if value { "yes" } else { "no" }
}
