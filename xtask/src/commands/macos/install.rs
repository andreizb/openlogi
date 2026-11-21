//! Transactional local installation. Persistent user state is never part of the transaction.

mod process;
#[cfg(test)]
mod tests;

use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result, bail};
use clap::Parser;
use tempfile::TempDir;
use xshell::{Shell, cmd};

use super::bundle;
use super::bundle::identity::{self, Channel, Component};
use crate::support::fs::repo_root;

#[derive(Parser)]
pub(crate) struct Args {
    /// Where the self-contained app is installed. Its parent must already be writable.
    #[arg(long, default_value = "/Applications/OpenLogi.app")]
    destination: PathBuf,
}

pub(super) fn run(args: &Args) -> Result<()> {
    if !cfg!(target_os = "macos") {
        bail!("macos install requires macOS");
    }
    if !args.destination.is_absolute()
        || args
            .destination
            .file_name()
            .is_none_or(|name| name != "OpenLogi.app")
    {
        bail!("--destination must be an absolute path ending in OpenLogi.app");
    }
    let parent = args
        .destination
        .parent()
        .context("missing installation parent")?;
    let scratch = tempfile::Builder::new()
        .prefix(".openlogi-install-")
        .tempdir_in(parent)
        .context("the installation folder must exist and be writable; run without sudo")?;
    let identity = bundle::identity_for_install(&args.destination)?;
    let before = process::health(&args.destination).ok();
    bundle::run_for_install(&identity)?;
    let built = repo_root()?.join("target/release/bundle/osx/OpenLogi.app");
    let staged = scratch.path().join("OpenLogi.app");
    let sh = Shell::new()?;
    cmd!(sh, "/usr/bin/ditto {built} {staged}").run()?;
    identity::verify(&staged, Channel::Production, Component::VARIANTS)?;
    cmd!(sh, "codesign --verify --deep --strict {staged}").run()?;
    bundle::verify_update_identity(&args.destination, &staged)?;
    process::stop(&args.destination)?;
    let replacement = Replacement::swap(scratch, &args.destination)?;
    let result = process::start(&args.destination)
        .and_then(|()| process::verify(&args.destination, before.as_ref()));
    if let Err(error) = result {
        let _ = process::stop(&args.destination);
        replacement.rollback().with_context(|| {
            format!("verification failed ({error:#}) and rollback could not complete")
        })?;
        if args.destination.exists() {
            let _ = process::start(&args.destination);
        }
        return Err(error)
            .context("the replacement failed verification; the previous app was restored");
    }
    replacement.commit()?;
    println!(
        "Installed and verified {}. Settings, credentials and runtime state were preserved.",
        args.destination.display()
    );
    Ok(())
}

use strum::VariantArray as _;

/// Owns the previous bundle until successful runtime verification makes it disposable.
struct Replacement {
    scratch: Option<TempDir>,
    destination: PathBuf,
    previous: Option<PathBuf>,
    disposition: Disposition,
}

enum Disposition {
    Pending,
    Verified,
}

impl Replacement {
    fn swap(scratch: TempDir, destination: &Path) -> Result<Self> {
        let previous = destination
            .exists()
            .then(|| scratch.path().join("Previous.app"));
        if let Some(previous) = &previous {
            fs_err::rename(destination, previous)?;
        }
        let pending = Self {
            scratch: Some(scratch),
            destination: destination.to_path_buf(),
            previous,
            disposition: Disposition::Pending,
        };
        let staged = pending
            .scratch
            .as_ref()
            .context("missing install staging directory")?
            .path()
            .join("OpenLogi.app");
        fs_err::rename(&staged, destination)?;
        Ok(pending)
    }

    fn commit(mut self) -> Result<()> {
        self.disposition = Disposition::Verified;
        self.scratch
            .take()
            .context("missing installation cleanup directory")?
            .close()
            .context(
                "the app is verified, but its temporary installation files could not be removed",
            )
    }

    fn rollback(mut self) -> Result<()> {
        self.restore()?;
        self.disposition = Disposition::Verified;
        Ok(())
    }

    fn restore(&self) -> Result<()> {
        let scratch = self
            .scratch
            .as_ref()
            .context("missing recovery directory")?;
        if self.destination.exists() {
            fs_err::rename(&self.destination, scratch.path().join("Failed.app"))?;
        }
        if let Some(previous) = &self.previous {
            fs_err::rename(previous, &self.destination)?;
        }
        Ok(())
    }
}

impl Drop for Replacement {
    fn drop(&mut self) {
        if matches!(self.disposition, Disposition::Pending)
            && let Err(error) = self.restore()
        {
            // Never delete the only recovery copy when a filesystem error blocks rollback.
            if let Some(scratch) = self.scratch.take() {
                eprintln!(
                    "Automatic restore failed: {error:#}. Recovery app retained at {}",
                    scratch.keep().display()
                );
            }
        }
    }
}
