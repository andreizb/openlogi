//! Signing the shipped bundle, inside out.
//!
//! Which identity is used and whether it carries a secure timestamp is the
//! difference between a build that notarizes and one that only runs locally,
//! so both are decided here rather than at the call site.

use std::env;
use std::path::{Path, PathBuf};

use anyhow::{Result, bail};
use strum::VariantArray as _;
use xshell::{Shell, cmd};

use super::identity::{Channel, Component};
use super::provisioning::AgentProfile;
use super::sparkle;
use crate::support::fs::{ensure_file, repo_root};

/// The Developer ID identity a release is signed with — read by the bundle
/// step here and by `macos dmg`'s `--sign-identity`.
pub(in crate::commands::macos) const SIGN_IDENTITY_ENV: &str = "OPENLOGI_SIGN_IDENTITY";

/// Select the existing certificate before building, so a local update retains TCC grants.
pub(in crate::commands::macos) fn identity_for_install(installed: &Path) -> Result<String> {
    if let Some(identity) =
        env_nonempty("OPENLOGI_LOCAL_CODESIGN_IDENTITY").or_else(|| env_nonempty(SIGN_IDENTITY_ENV))
    {
        return Ok(identity);
    }
    if installed.exists() {
        let sh = Shell::new()?;
        let output = cmd!(sh, "codesign -dv --verbose=4 {installed}").output()?;
        if !output.status.success() {
            bail!("the installed app has no readable signing identity; it was left untouched");
        }
        if let Some(identity) = first_authority(&String::from_utf8_lossy(&output.stderr)) {
            return Ok(identity.to_owned());
        }
        bail!(
            "the installed app is ad-hoc signed; select OPENLOGI_LOCAL_CODESIGN_IDENTITY explicitly"
        );
    }
    first_apple_development_identity()?.ok_or_else(|| anyhow::anyhow!(
        "no local signing certificate found; add an Apple Development certificate in Xcode first"
    ))
}

fn first_authority(output: &str) -> Option<&str> {
    output
        .lines()
        .find_map(|line| line.strip_prefix("Authority="))
}

fn designated_requirement(app: &Path) -> Result<String> {
    let sh = Shell::new()?;
    let output = cmd!(sh, "codesign -dr - {app}").output()?;
    if !output.status.success() {
        bail!("could not read {} signing requirement", app.display());
    }
    let text = format!(
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    text.lines()
        .find(|line| line.starts_with("designated => "))
        .map(str::to_owned)
        .ok_or_else(|| anyhow::anyhow!("missing designated requirement for {}", app.display()))
}

/// Refuse to replace an app when any component would lose its existing privacy identity.
pub(in crate::commands::macos) fn verify_update_identity(
    installed: &Path,
    replacement: &Path,
) -> Result<()> {
    if !installed.exists() {
        return Ok(());
    }
    for component in Component::VARIANTS {
        let old = component.root(installed, Channel::Production);
        let new = component.root(replacement, Channel::Production);
        if designated_requirement(&old)? != designated_requirement(&new)? {
            bail!("{component} signing identity differs; the installed app was left untouched");
        }
    }
    Ok(())
}

pub(super) fn local_sign_app_if_available(channel: Channel) -> Result<()> {
    if env::var("OPENLOGI_LOCAL_CODESIGN").as_deref() == Ok("0") {
        println!("==> local codesign: skipped (OPENLOGI_LOCAL_CODESIGN=0)");
        return Ok(());
    }

    if let Some(identity) = env_nonempty(SIGN_IDENTITY_ENV) {
        sign_app_with_timestamp(&identity, TimestampMode::Secure, channel, None)?;
        return Ok(());
    }

    if let Some(identity) = env_nonempty("OPENLOGI_LOCAL_CODESIGN_IDENTITY") {
        sign_app_with_timestamp(&identity, TimestampMode::None, channel, None)?;
        return Ok(());
    }

    if let Some(identity) = first_apple_development_identity()? {
        sign_app_with_timestamp(&identity, TimestampMode::None, channel, None)?;
        return Ok(());
    }

    println!(
        "==> local codesign: skipped (no Apple Development identity found;          set OPENLOGI_LOCAL_CODESIGN_IDENTITY or {SIGN_IDENTITY_ENV} to sign)"
    );
    println!(
        "    warning: an unsigned bundle is re-signed ad-hoc on every build, so its own Accessibility grant goes stale each time"
    );
    Ok(())
}

/// Sign the bundle inside out. Given `agent_profile`, the agent helper embeds
/// it and claims the HID virtual-device entitlement it grants; without one,
/// the helper claims no entitlements, because a claim without its profile
/// gets the agent killed at launch.
pub(super) fn sign_app_with_timestamp(
    identity: &str,
    timestamp: TimestampMode,
    channel: Channel,
    agent_profile: Option<&AgentProfile>,
) -> Result<()> {
    let sh = Shell::new()?;
    let root = repo_root()?;
    let app = root.join("target/release/bundle/osx/OpenLogi.app");
    let helper = Component::Agent.root(&app, channel);
    let overlay = Component::Overlay.root(&app, channel);
    // GUI + embedded CLI open the camera (preview / snapshot). The agent and
    // overlay helpers do not — leave them without camera entitlements.
    let camera_ents = camera_entitlements_path(&root);
    ensure_file(&camera_ents)?;
    println!("==> codesign ({identity})");
    // Inside-out signing: seal the nested helper with its own signature first,
    // then the outer app (which seals the already-signed helper). `--deep` is
    // deprecated and can't give the helper an independent signature — but a
    // stable, separately-signed helper identity is exactly what lets the agent's
    // Accessibility (TCC) grant persist across updates. So sign each explicitly.
    match (agent_profile, helper.exists()) {
        (Some(profile), true) => {
            let scratch = tempfile::tempdir()?;
            let entitlements = profile.install(&helper, scratch.path())?;
            codesign_runtime(identity, &helper, timestamp, Some(&entitlements))?;
            profile.verify_signed(&helper, scratch.path())?;
        }
        (Some(_), false) => bail!(
            "the agent helper is missing at {}, so its provisioning profile has nowhere to go",
            helper.display()
        ),
        (None, true) => codesign_runtime(identity, &helper, timestamp, None)?,
        (None, false) => {}
    }
    if overlay.exists() {
        codesign_runtime(identity, &overlay, timestamp, None)?;
    }
    // Sparkle ships ad-hoc signed. The app's hardened runtime only loads a
    // framework signed by its own team, so re-sign Sparkle's helpers and then
    // the framework that seals them, before the app seals the framework.
    let sparkle = sparkle::code_inside_out(&app);
    for code in &sparkle {
        codesign_runtime(identity, code, timestamp, None)?;
    }
    // The embedded CLI is a second Mach-O under Contents/MacOS; sign it with the
    // hardened runtime before the outer app so it carries a Developer ID
    // signature (its as-built ad-hoc signature would fail notarization).
    let cli = app.join("Contents/MacOS/openlogi");
    if cli.exists() {
        codesign_runtime(identity, &cli, timestamp, Some(&camera_ents))?;
    }
    codesign_runtime(identity, &app, timestamp, Some(&camera_ents))?;
    cmd!(sh, "codesign --verify --strict {app}").run()?;
    if helper.exists() {
        cmd!(sh, "codesign --verify --strict {helper}").run()?;
    }
    if overlay.exists() {
        cmd!(sh, "codesign --verify --strict {overlay}").run()?;
    }
    if let Some(framework) = sparkle.last() {
        // `--deep` is fine for verifying (it is deprecated only for signing):
        // it checks every nested signature the framework seals.
        cmd!(sh, "codesign --verify --deep --strict {framework}").run()?;
    }
    if cli.exists() {
        cmd!(sh, "codesign --verify --strict {cli}").run()?;
    }
    Ok(())
}

/// Path to the GUI/CLI entitlements (camera hardened-runtime exception).
fn camera_entitlements_path(root: &Path) -> PathBuf {
    root.join("crates/openlogi-desktop/bundle/OpenLogi.entitlements")
}

/// Sign one target with the hardened runtime and the requested timestamp mode.
fn codesign_runtime(
    identity: &str,
    target: &Path,
    timestamp: TimestampMode,
    entitlements: Option<&Path>,
) -> Result<()> {
    let sh = Shell::new()?;
    match (timestamp, entitlements) {
        (TimestampMode::Secure, Some(ents)) => {
            cmd!(
                sh,
                "codesign --force --options runtime --timestamp --entitlements {ents} --sign {identity} {target}"
            )
            .run()?;
        }
        (TimestampMode::Secure, None) => {
            cmd!(
                sh,
                "codesign --force --options runtime --timestamp --sign {identity} {target}"
            )
            .run()?;
        }
        (TimestampMode::None, Some(ents)) => {
            cmd!(
                sh,
                "codesign --force --options runtime --timestamp=none --entitlements {ents} --sign {identity} {target}"
            )
            .run()?;
        }
        (TimestampMode::None, None) => {
            cmd!(
                sh,
                "codesign --force --options runtime --timestamp=none --sign {identity} {target}"
            )
            .run()?;
        }
    }
    Ok(())
}

#[derive(Clone, Copy)]
pub(super) enum TimestampMode {
    Secure,
    None,
}

fn env_nonempty(name: &str) -> Option<String> {
    env::var(name).ok().filter(|value| !value.trim().is_empty())
}

fn first_apple_development_identity() -> Result<Option<String>> {
    let sh = Shell::new()?;
    let Ok(output) = cmd!(sh, "security find-identity -v -p codesigning").read() else {
        return Ok(None);
    };
    Ok(output
        .lines()
        .filter_map(quoted_identity)
        .find(|identity| identity.starts_with("Apple Development:")))
}

pub(crate) fn quoted_identity(line: &str) -> Option<String> {
    let start = line.find('"')? + 1;
    let end = line[start..].find('"')?;
    Some(line[start..start + end].to_string())
}

#[cfg(test)]
mod tests;
