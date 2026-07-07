//! Root-only integration test: start a real host-based container and
//! check isolation end-to-end.
//!
//! Run with: `sudo -E cargo test -p orca-container -- --ignored`

use orca_container::{ContainerBuilder, IoMode, RunAs, SessionPaths};
use orca_image::{Base, Image, ImageConfig, Layer, Upper};

const TEST_PATH: &str = "PATH=/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin";

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

fn host_image(upper: &std::path::Path) -> Image {
    Image {
        upper: Upper::new(upper.to_path_buf()),
        lower: Vec::<Layer>::new(),
        base: Base::Host,
        config: ImageConfig::default(),
    }
}

#[test]
#[ignore = "requires root (mount / clone / pivot_root)"]
fn host_container_isolates_writes_and_propagates_exit_code() {
    let dir = tempfile::tempdir().unwrap();
    let upper = dir.path().join("diff");
    std::fs::create_dir_all(&upper).unwrap();

    let marker = "orca-container-test-marker";
    let container = ContainerBuilder::new(host_image(&upper), session_in(dir.path()))
        .io(IoMode::Piped)
        .hostname("itest")
        .env(vec![TEST_PATH.into()])
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
        config: ImageConfig::default(),
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

#[test]
#[ignore = "requires root (mount / clone / pivot_root / setresuid)"]
fn run_as_drops_all_ids_completely() {
    let dir = tempfile::tempdir().unwrap();
    let upper = dir.path().join("diff");
    std::fs::create_dir_all(&upper).unwrap();

    // 65534 = nobody/nogroup on common distros; the exact name does not
    // matter since we check numeric ids only.
    let container = ContainerBuilder::new(host_image(&upper), session_in(dir.path()))
        .io(IoMode::Piped)
        .env(vec![TEST_PATH.into()])
        .run_as(RunAs {
            uid: 65534,
            gid: 65534,
            groups: vec![65534],
        })
        .cmd(vec![
            "/bin/sh".into(),
            "-c".into(),
            // Real, effective and saved ids must all be dropped, and the
            // 666 devices must be writable by the unprivileged user.
            "[ \"$(id -u)\" = 65534 ] && [ \"$(id -g)\" = 65534 ] \
             && [ \"$(id -ur)\" = 65534 ] \
             && echo x > /dev/null && exit 9"
                .into(),
        ])
        .build()
        .unwrap();
    let done = container.run().unwrap().wait().unwrap();
    assert_eq!(done.status(), 9);
}
