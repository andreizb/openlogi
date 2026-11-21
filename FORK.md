# Spotlight fork

This is a fork of [AprilNEA/OpenLogi](https://github.com/AprilNEA/OpenLogi), with
Logitech Spotlight presenter support. Upstream project history, contributor
attribution, [MIT](LICENSE-MIT) and [Apache-2.0](LICENSE-APACHE) licenses remain
intact. The upstream `design/` assets retain their separate proprietary terms.

The presenter additions include highlight, magnification and digital laser;
effect ordering and double-click switching; pointer speed, size, contrast, color
and magnification controls; Next/Back hold actions; per-application settings;
timers and vibration. Hardware behavior needs a real presenter to verify.

## Install or update on macOS

Review [run.sh](run.sh), then run:

```sh
curl -fsSL https://raw.githubusercontent.com/andreizb/openlogi/feat/spotlight-presenter/run.sh | bash
```

The same command installs a missing app or updates an existing one. It downloads
this branch into a new temporary folder, builds and signs the app, installs it
in `/Applications/OpenLogi.app`, and checks the running GUI, background agent,
overlay, code signatures and agent connection. It removes the downloaded source
and build outputs on success, failure or interruption. A failure after replacing
the app restores its previous bundle. If a filesystem failure blocks restoration,
the runner retains and reports the recovery directory instead of deleting it.

Requirements:

- macOS with full Xcode 26+ selected, including its Metal toolchain. Command Line
  Tools alone cannot build GPUI's shaders.
- Homebrew. Missing Rust and `cargo-bundle` tools are installed through Homebrew.
  Those tools and shared Cargo download caches are retained for future runs.
  Disposable builds default to ThinLTO and four parallel build jobs; explicit
  `CARGO_PROFILE_RELEASE_LTO` and `CARGO_BUILD_JOBS` values override those defaults.
- A local Apple Development or Developer ID signing certificate. Xcode can create
  an Apple Development certificate. For an existing app the runner selects its
  current certificate and refuses a different app/helper signing requirement.
  The private key must remain in your local Keychain; it never belongs in Git.

To select a certificate explicitly, use `OPENLOGI_LOCAL_CODESIGN_IDENTITY`.
`OPENLOGI_REF` selects another published branch or tag. These are environment
variables for the `bash` process executing the runner. Run without `sudo`.

The app, agent and overlay all run from the installed bundle, independently of
the source folder. The runner does not edit user configuration, pairing keys,
credentials, device caches or other runtime state. Existing permissions are
preserved by matching signing identities; a first installation still needs its
normal macOS permissions. It neither resets TCC nor grants permissions itself.

Verify independently at any time:

```sh
/Applications/OpenLogi.app/Contents/MacOS/openlogi api status
codesign --verify --deep --strict /Applications/OpenLogi.app
```

`api status` proves the service connection and reports permissions; it does not
prove physical presenter effects, button input or vibration.

## Upstream and publication

The runner installs this reviewed branch; it does not merge upstream on your
computer. Before publishing an upstream sync, check the latest release, current
source and [Spotlight issue #279](https://github.com/AprilNEA/OpenLogi/issues/279).
The canonical stop rule is in [AGENTS.md](AGENTS.md#spotlight-fork-update-gate):
when functional Spotlight support lands upstream, stop fork replacements and
report its scope. Receiver recognition alone does not meet that condition.

Keep tests synthetic and never commit local configuration, hardware identities,
signing keys, logs, caches or build directories. Git attribution and upstream
license notices are intentional public history. Test the published runner from
a fresh GitHub download before deleting a development checkout; confirm every
local commit is present on GitHub first.

The dependency audit records [RUSTSEC-2026-0542](https://rustsec.org/advisories/RUSTSEC-2026-0542.html)
as a reviewed maintenance exception: tarpc uses the unmaintained `humantime`
crate to format deadline logs, and no patched version exists. The exception is
specific to that notice; vulnerability checks stay enabled.
