//! End-to-end policy tests. Run with root privileges and mount namespaces enabled.
use orca::{BaseImageRef, Orca, Settings};

#[test]
#[ignore = "requires root and mount namespaces"]
fn target_passwd_config_and_final_path_are_used() {
    let dir = tempfile::tempdir().unwrap();
    let orca = Orca::new(dir.path());
    let mut store = orca.envs().unwrap();
    let env = store
        .create("configured".into(), BaseImageRef::Host)
        .unwrap();
    let etc = env.upper_path().join("etc");
    std::fs::create_dir_all(&etc).unwrap();
    std::fs::write(
        etc.join("passwd"),
        "orca-test:x:12345:12346::/configured-home:/bin/sh\n",
    )
    .unwrap();
    std::fs::write(
        etc.join("group"),
        "primary:x:12346:\nextra:x:12347:orca-test\n",
    )
    .unwrap();
    use std::os::unix::fs::PermissionsExt;
    let bin = env.upper_path().join("orca-test-bin");
    std::fs::create_dir_all(&bin).unwrap();
    let probe = bin.join("orca-env-probe");
    std::fs::write(&probe, "#!/bin/sh\nexec /bin/sh \"$@\"\n").unwrap();
    std::fs::set_permissions(probe, std::fs::Permissions::from_mode(0o755)).unwrap();
    std::fs::write(
        dir.path().join("config.toml"),
        r#"
[global]
working_dir = "/"
[global.env]
PATH = "/orca-test-bin:/bin:/usr/bin"
KEPT = "global"
[envs.configured]
user = "orca-test"
[envs.configured.env]
HOME = "/explicit-home"
"#,
    )
    .unwrap();
    let args = Settings {
        cmd: Some(vec!["orca-env-probe".into(), "-c".into(),
            "test \"$(id -u)\" = 12345 && test \"$(id -g)\" = 12346 && test \"$HOME\" = /explicit-home && test \"$USER\" = orca-test && test \"$KEPT\" = global && test \"$PWD\" = / && test -z \"${TERM+x}\"".into()]),
        ..Default::default()
    };
    let settings = orca.config().unwrap().resolve(&env.name, &args).unwrap();
    assert_eq!(env.image().unwrap().run(settings).unwrap(), 0);
    let bad = Settings {
        user: Some("no-such-orca-test-user".into()),
        cmd: Some(vec!["/bin/true".into()]),
        ..Default::default()
    };
    let err = env.image().unwrap().run(bad).unwrap_err().to_string();
    assert!(err.contains("user not found"), "{err}");
    assert!(!env.session_path().exists());
}

#[test]
#[ignore = "requires root and mount namespaces"]
fn explicit_missing_cwd_aborts_and_numeric_user_needs_no_entry() {
    let dir = tempfile::tempdir().unwrap();
    let orca = Orca::new(dir.path());
    let mut store = orca.envs().unwrap();
    let env = store.create("numeric".into(), BaseImageRef::Host).unwrap();
    let etc = env.upper_path().join("etc");
    std::fs::create_dir_all(&etc).unwrap();
    std::fs::write(etc.join("passwd"), "").unwrap();
    std::fs::write(etc.join("group"), "").unwrap();
    let settings = Settings {
        user: Some("12345".into()),
        group: Some("12346".into()),
        working_dir: Some("/".into()),
        cmd: Some(vec![
            "/bin/sh".into(),
            "-c".into(),
            "test \"$(id -u)\" = 12345 && test \"$(id -g)\" = 12346 && test \"$HOME\" = /".into(),
        ]),
        ..Default::default()
    };
    assert_eq!(env.image().unwrap().run(settings.clone()).unwrap(), 0);
    let bad = Settings {
        working_dir: Some("/orca-definitely-missing-cwd".into()),
        ..settings
    };
    assert!(
        env.image()
            .unwrap()
            .run(bad)
            .unwrap_err()
            .to_string()
            .contains("chdir")
    );
}
