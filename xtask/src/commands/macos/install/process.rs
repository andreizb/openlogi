//! Only processes executing inside the installed bundle are restarted.

use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use anyhow::{Context as _, Result, bail};
use serde::Deserialize;
use sysinfo::{ProcessRefreshKind, ProcessesToUpdate, Signal, System, UpdateKind};
use xshell::{Shell, cmd};

use super::super::bundle::{
    self,
    identity::{Channel, Component},
};
use openlogi_core::brand;

const COMMAND_TIMEOUT: Duration = Duration::from_secs(4);
const START_TIMEOUT: Duration = Duration::from_secs(30);
const POLL: Duration = Duration::from_millis(200);

#[derive(Deserialize)]
struct Envelope {
    ok: bool,
    data: Option<Health>,
}

#[derive(Deserialize)]
pub(super) struct Health {
    agent_version: String,
    inventory: String,
    accessibility_granted: bool,
    input_monitoring_granted: bool,
    hook_installed: bool,
}

fn snapshot() -> System {
    let mut system = System::new();
    system.refresh_processes_specifics(
        ProcessesToUpdate::All,
        true,
        ProcessRefreshKind::nothing().with_exe(UpdateKind::Always),
    );
    system
}

pub(super) fn stop(app: &Path) -> Result<()> {
    let system = snapshot();
    let pids: Vec<_> = system
        .processes()
        .iter()
        .filter_map(|(pid, process)| {
            process
                .exe()
                .is_some_and(|exe| exe.starts_with(app))
                .then_some(*pid)
        })
        .collect();
    for pid in &pids {
        if let Some(process) = system.process(*pid) {
            let _ = process.kill_with(Signal::Term);
        }
    }
    let started = Instant::now();
    while started.elapsed() < COMMAND_TIMEOUT {
        let mut system = System::new();
        system.refresh_processes(ProcessesToUpdate::Some(&pids), true);
        if pids.iter().all(|pid| system.process(*pid).is_none()) {
            return Ok(());
        }
        std::thread::sleep(POLL);
    }
    bail!("an installed OpenLogi process did not exit; the app was left untouched")
}

pub(super) fn start(app: &Path) -> Result<()> {
    let sh = Shell::new()?;
    let uid = cmd!(sh, "/usr/bin/id -u").read()?;
    let label = bundle::agent_service_label(Channel::Production);
    let service = format!("gui/{uid}/{label}");
    // Keep existing SMAppService registration and Background App Activity approval.
    // For a never-registered installation, LaunchServices gives the helper its own TCC identity.
    let restarted = cmd!(sh, "/bin/launchctl kickstart -k {service}")
        .quiet()
        .ignore_status()
        .output()?;
    if !restarted.status.success() {
        let agent = Component::Agent.root(app, Channel::Production);
        cmd!(sh, "/usr/bin/open -g -n {agent}").run()?;
    }
    cmd!(sh, "/usr/bin/open -g {app}").run()?;
    Ok(())
}

pub(super) fn health(app: &Path) -> Result<Health> {
    let cli = app.join("Contents/MacOS").join(brand::CLI_EXECUTABLE);
    let mut child = Command::new(cli)
        .args(["api", "status"])
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()?;
    let started = Instant::now();
    loop {
        if child.try_wait()?.is_some() {
            break;
        }
        if started.elapsed() >= COMMAND_TIMEOUT {
            let _ = child.kill();
            let _ = child.wait();
            bail!("the installed command-line status check timed out");
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    let output = child.wait_with_output()?;
    let envelope: Envelope =
        serde_json::from_slice(&output.stdout).context("invalid agent status response")?;
    if !output.status.success() || !envelope.ok {
        bail!("the new app could not reach its background service");
    }
    envelope.data.context("missing agent status")
}

fn permissions_preserved(before: Option<&Health>, after: &Health) -> bool {
    before.is_none_or(|before| {
        (!before.accessibility_granted || after.accessibility_granted)
            && (!before.input_monitoring_granted || after.input_monitoring_granted)
            && (!before.hook_installed || after.hook_installed)
    })
}

pub(super) fn verify(
    app: &Path,
    before: Option<&Health>,
    cancellation: &super::Cancellation,
) -> Result<()> {
    let started = Instant::now();
    let mut reason = String::from("waiting for the new agent");
    while started.elapsed() < START_TIMEOUT {
        cancellation.check()?;
        match health(app) {
            Ok(after)
                if after.agent_version == env!("CARGO_PKG_VERSION")
                    && after.inventory == "ready"
                    && permissions_preserved(before, &after) =>
            {
                let system = snapshot();
                let names = [
                    brand::GUI_EXECUTABLE,
                    brand::Helper::Agent.executable(),
                    brand::Helper::Overlay.executable(),
                ];
                if names.iter().all(|name| {
                    system.processes().values().any(|process| {
                        process.name() == *name
                            && process.exe().is_some_and(|exe| exe.starts_with(app))
                    })
                }) {
                    let sh = Shell::new()?;
                    cmd!(sh, "codesign --verify --deep --strict {app}").run()?;
                    return Ok(());
                }
                reason = "the GUI, agent and overlay must all run from the installed app".into();
            }
            Ok(_) => {
                reason = "the new agent is not ready or an existing permission was lost".into();
            }
            Err(error) => reason = format!("{error:#}"),
        }
        std::thread::sleep(POLL);
    }
    bail!("installed app verification failed: {reason}")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn health_with_permissions(granted: bool) -> Health {
        Health {
            agent_version: "test".into(),
            inventory: "ready".into(),
            accessibility_granted: granted,
            input_monitoring_granted: granted,
            hook_installed: granted,
        }
    }

    #[test]
    fn an_update_cannot_claim_success_after_losing_existing_permissions() {
        let granted = health_with_permissions(true);
        let ungranted = health_with_permissions(false);
        assert!(!permissions_preserved(Some(&granted), &ungranted));
        assert!(permissions_preserved(Some(&granted), &granted));
        assert!(permissions_preserved(None, &ungranted));
    }
}
