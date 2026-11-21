use super::*;

fn staged(parent: &Path) -> TempDir {
    let scratch = tempfile::tempdir_in(parent).unwrap();
    fs_err::create_dir(scratch.path().join("OpenLogi.app")).unwrap();
    fs_err::write(scratch.path().join("OpenLogi.app/version"), "new").unwrap();
    scratch
}

#[test]
fn verification_failure_restores_the_previous_app_without_touching_user_state() {
    let parent = tempfile::tempdir().unwrap();
    let app = parent.path().join("OpenLogi.app");
    fs_err::create_dir(&app).unwrap();
    fs_err::write(app.join("version"), "old").unwrap();
    let config = parent.path().join(openlogi_core::paths::CONFIG_FILE);
    fs_err::write(&config, "private runtime state").unwrap();
    {
        let _unverified = Replacement::swap(staged(parent.path()), &app).unwrap();
        assert_eq!(fs_err::read_to_string(app.join("version")).unwrap(), "new");
    }
    assert_eq!(fs_err::read_to_string(app.join("version")).unwrap(), "old");
    assert_eq!(
        fs_err::read_to_string(config).unwrap(),
        "private runtime state"
    );
}

#[test]
fn a_verified_install_survives_the_temporary_directory() {
    let parent = tempfile::tempdir().unwrap();
    let app = parent.path().join("OpenLogi.app");
    let scratch = staged(parent.path());
    let path = scratch.path().to_path_buf();
    Replacement::swap(scratch, &app).unwrap().commit().unwrap();
    assert!(!path.exists());
    assert_eq!(fs_err::read_to_string(app.join("version")).unwrap(), "new");
}

#[test]
fn a_failed_first_install_leaves_no_unverified_app() {
    let parent = tempfile::tempdir().unwrap();
    let app = parent.path().join("OpenLogi.app");
    drop(Replacement::swap(staged(parent.path()), &app).unwrap());
    assert!(!app.exists());
}

#[test]
fn interruption_after_replacement_restores_the_previous_app() {
    let parent = tempfile::tempdir().unwrap();
    let app = parent.path().join("OpenLogi.app");
    fs_err::create_dir(&app).unwrap();
    fs_err::write(app.join("version"), "old").unwrap();
    let cancellation = Cancellation::default();
    let replacement = Replacement::swap(staged(parent.path()), &app).unwrap();
    cancellation.cancel();
    cancellation
        .check()
        .expect_err("interruption must stop installation");
    replacement.rollback().unwrap();
    assert_eq!(fs_err::read_to_string(app.join("version")).unwrap(), "old");
}
