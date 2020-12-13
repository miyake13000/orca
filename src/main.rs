// orca : CLI Container Runtime
// This program is created by nomlab in Okayama University
// author nomlab <https://github.com/nomlab>
//        miyake13000 <https://github.com/miyake13000/crca>

#[macro_use]
extern crate clap;
extern crate nix;
extern crate libc;
use nix::{sched, unistd, mount, sys};
use std::ffi::{CStr, CString};
use std::fs;
use std::io::Write;
use clap::{App, Arg};

extern crate reqwest;
extern crate serde;
extern crate serde_json;
use serde::{Deserialize, Serialize};
use std::io;
use std::fs::{File, create_dir_all};
use std::path::{Path, PathBuf};
use flate2::read::GzDecoder;
use tar::Archive;
use dirs::home_dir;

#[derive(Serialize, Deserialize)]
#[allow(non_snake_case)]
struct Res1 {
    token: String,
    access_token: String,
    expires_in: u32,
    issued_at: String,
}

#[derive(Serialize, Deserialize)]
#[allow(non_snake_case)]
struct Config {
    mediaType: String,
    size: usize,
    digest: String,
}

#[derive(Serialize, Deserialize)]
#[allow(non_snake_case)]
struct Res2 {
    schemaVersion: usize,
    mediaType: String,
    config: Config,
    layers: Vec<Config>,
}

pub struct Image {
    dest_name: String,
    dest_tag:  String,
    path: PathBuf,
}

impl Image {
    pub fn new(name: String, tag: String) -> Image {
        let mut path = PathBuf::new();
        let home_dir = home_dir().unwrap();
        path.push(home_dir);
        path.push(".local/orca/containers");
        path.push(&name);
        path.push(&tag);

        Image{dest_name: name, dest_tag: tag, path: path}
    }

    pub fn exist(&self) -> bool {
        Path::new(self.path.as_path()).exists()
    }

    pub fn get(&self) -> Result<(), ()> {
        let url = format!("https://auth.docker.io/token?service=registry.docker.io&scope=repository:library/{}:pull", self.dest_name);
        let client = reqwest::blocking::Client::new();
        let resp = client.get(&url)
            .send()
            .unwrap()
            .text()
            .unwrap();
        let res_json: Res1 = serde_json::from_str(&resp).unwrap();
        let token = &res_json.token;

        let url = format!("https://registry-1.docker.io/v2/library/{}/manifests/{}", self.dest_name, self.dest_tag);
        let client = reqwest::blocking::Client::new();
        let resp = client.get(&url)
            .header(reqwest::header::ACCEPT, "application/vnd.docker.distribution.manifest.v2+json")
            .bearer_auth(token)
            .send()
            .unwrap()
            .text()
            .unwrap();
        let res_json: Res2 = serde_json::from_str(&resp).unwrap();
        let layer_id = &res_json.layers[0].digest;

        let url = format!("https://registry-1.docker.io/v2/library/{}/blobs/{}", self.dest_name, layer_id);
        let client = reqwest::blocking::Client::new();
        let mut resp = client.get(&url)
            .bearer_auth(token)
            .send()
            .unwrap();

        create_dir_all(self.path.as_path()).expect("create directory");
        let path = format!("{}/image.tar.gz", self.path.to_str().unwrap());
        let mut file = File::create(path).expect("file create");
        io::copy(&mut resp, &mut file).expect("copy");
        Ok(())
    }

    pub fn extract(&self) -> Result<(), ()> {
        let path = format!("{}/image.tar.gz", self.path.to_str().unwrap());
        let tar_gz = File::open(path).expect("file open");
        let tar = GzDecoder::new(tar_gz);
        let mut archive = Archive::new(tar);
        let path = format!("{}/rootfs", self.path.to_str().unwrap());
        archive.unpack(path).expect("unpack");
        Ok(())
    }
}

fn main() {
    let input = get_input();
    let matches = input.get_matches();
    let command = formatter(&matches);

    let image = Image::new(String::from("debian"), String::from("latest"));
    if  !image.exist() {
        println!("Cannot find container image on local");
        println!("Downloading container image...");
        image.get().unwrap();
        println!("Extracting container image...");
        image.extract().unwrap();
        println!("done");
    }

    const STACK_SIZE: usize = 1024 * 1024;
    let ref mut stack: [u8; STACK_SIZE] = [0; STACK_SIZE];
    let cb = Box::new(|| child(command, &image));

    let pid = clone(cb, stack).expect("clone");
    let pid_int = pid.as_raw() as i32;
    let uid = unistd::getuid().as_raw() as u32;
    id_map(pid_int, 0, uid, 1).expect("set_uid");
    sys::wait::wait().expect("wait");
}

fn child(command: &str, image: &Image) -> isize {
    let path = format!("{}/rootfs", image.path.to_str().unwrap());
    let path_buf = PathBuf::from(path);
    unistd::chdir(path_buf.as_path()).expect("chdir");
    unistd::chroot(path_buf.as_path()).expect("chroot");
    unistd::sethostname(&image.dest_name).expect("sethostname");

    create_dir_all("/proc").unwrap();
    mount("proc", "/proc", "proc", "").expect("mount proc");
    create_dir_all("/dev/pts").unwrap();
    mount("devpts", "/dev/pts", "devpts", "").expect("mount devpts");

    let mut argv: Vec<&CStr> = Vec::new();

    let command_cstring = CString::new(command).expect("CString::new");
    let command_cstr = CStr::from_bytes_with_nul(command_cstring
                                              .to_bytes_with_nul())
                                              .expect("CString to CStr");
    argv.push(command_cstr);

    let mut envp: Vec<&CStr> = Vec::new();
    envp.push(CStr::from_bytes_with_nul(b"SHELL=/bin/bash\0").expect("env shell"));
    envp.push(CStr::from_bytes_with_nul(b"HOME=/root\0").expect("env home"));
    envp.push(CStr::from_bytes_with_nul(b"TERM=xterm-256color\0").expect("env term"));

    unistd::execvpe(command_cstr, &argv, &envp).expect("execvpe");

    return 0;
}

fn id_map(pid: i32, innner_id: u32, outer_id: u32, lenge: u32) -> std::io::Result<usize> {
    let path = format!("{}{}", "/proc/", pid.to_string());
    let path_uid = format!("{}{}", path, "/uid_map");
    let path_gid = format!("{}{}", path, "/gid_map");
    let path_setg = format!("{}{}", path, "/setgroups");
    let content = format!("{} {} {}", innner_id, outer_id, lenge);
    let mut file_uid = fs::File::create(path_uid).unwrap();
    let mut file_gid = fs::File::create(path_gid).unwrap();
    let mut file_setg = fs::File::create(path_setg).unwrap();
    file_uid.write(content.as_bytes()).expect("write uid");
    file_setg.write(b"deny").expect("write setg");
    file_gid.write(content.as_bytes())
}

fn clone(cb: sched::CloneCb, stack: &mut [u8]) -> nix::Result<unistd::Pid> {

    let mut flags = sched::CloneFlags::empty();
    flags.insert(sched::CloneFlags::CLONE_NEWUSER);
    flags.insert(sched::CloneFlags::CLONE_NEWUTS);
    flags.insert(sched::CloneFlags::CLONE_NEWIPC);
    flags.insert(sched::CloneFlags::CLONE_NEWPID);
    flags.insert(sched::CloneFlags::CLONE_NEWNET);
    flags.insert(sched::CloneFlags::CLONE_NEWNS);

    let signal = Some(libc::SIGCHLD);

    sched::clone(cb, stack, flags, signal)
}

fn mount(src: &str, trg: &str, fstyp: &str, data: &str) -> nix::Result<()> {
    mount::mount(Some(src),
                 trg,
                 Some(fstyp),
                 mount::MsFlags::empty(),
                 Some(data))
}

fn get_input() -> App<'static, 'static> {
    let app = App::new(crate_name!())
        .version(crate_version!())
        .author(crate_authors!())
        .about(crate_description!())
        .arg(Arg::with_name("command")
             .help("command to execute in conainer")
             .required(false)
            );
    return app
}

fn formatter<'a>(matches: &'a clap::ArgMatches) -> &'a str {
    if let Some(o) = matches.value_of("command") {
        o
    }else {
        "/bin/bash"
    }
}

