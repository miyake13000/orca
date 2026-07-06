//! Root-only integration test: start a real host-based container and
//! check isolation end-to-end.
//!
//! Run with: `sudo -E cargo test -p orca-container -- --ignored`

use orca_container::{ContainerBuilder, IoMode, SessionPaths};
use orca_image::{Base, Image, ImageConfig, Layer, Upper};

fn session_in(dir: &std::path::Path) -> SessionPaths {
    let base = dir.join("session");
    SessionPaths {
        rootfs: base.join("rootfs"),
        work: base.join("work"),
        fake_rootfs: base.join("fake_rootfs"),
        fake_upper: base.join("fake_upper"),
        fake_work: base.join("fake_work"),
        base,
    }
}

#[test]
#[ignore = "requires root (mount / clone / pivot_root)"]
fn host_container_isolates_writes_and_propagates_exit_code() {
    let dir = tempfile::tempdir().unwrap();
    let upper = dir.path().join("diff");
    std::fs::create_dir_all(&upper).unwrap();

    let image = Image {
        upper: Upper::new(upper.clone()),
        lower: Vec::<Layer>::new(),
        base: Base::Host,
        config: ImageConfig::host_default(),
    };

    let marker = "orca-container-test-marker";
    let container = ContainerBuilder::new(image, session_in(dir.path()))
        .io(IoMode::Piped)
        .hostname("itest")
        .cmd(vec![
            "/bin/sh".into(),
            "-c".into(),
            // Also assert device permissions: mknod is umask-sensitive,
            // so /dev/null must be forced back to 666 (regression test).
            format!(
                "echo data > /{marker} && [ \"$(hostname)\" = itest ] \
                 && [ \"$(stat -c %a /dev/null)\" = 666 ] \
                 && [ \"$(stat -c %a /dev/tty)\" = 666 ] && exit 7"
            ),
        ])
        .build()
        .unwrap();

    let done = container.run().unwrap().wait().unwrap();
    // Exit code propagated (7 = both the write and the hostname check ran).
    assert_eq!(done.status(), 7);
    // The write landed in the upper, not on the host.
    assert!(upper.join(marker).is_file());
    assert!(!std::path::Path::new("/").join(marker).exists());
    // The session directory was cleaned up.
    assert!(!dir.path().join("session").exists());
}

#[test]
#[ignore = "requires root (mount / clone / pivot_root)"]
fn init_failure_is_reported_not_hung() {
    let dir = tempfile::tempdir().unwrap();
    let upper = dir.path().join("diff");
    std::fs::create_dir_all(&upper).unwrap();

    // A lower that does not exist makes the overlay mount fail; the child
    // must report the stage over the status pipe instead of hanging.
    let image = Image {
        upper: Upper::new(upper),
        lower: vec![Layer::new(dir.path().join("no-such-layer"))],
        base: Base::Host,
        config: ImageConfig::host_default(),
    };
    let container = ContainerBuilder::new(image, session_in(dir.path()))
        .io(IoMode::Piped)
        .cmd(vec!["/bin/true".into()])
        .build()
        .unwrap();
    let err = container.run().err().expect("run must fail");
    let msg = err.to_string();
    assert!(msg.contains("initialization failed"), "got: {msg}");
    assert!(!dir.path().join("session").exists());
}
