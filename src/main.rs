// This program is created by nomlab in Okayama University
// https://github.com/miyake13000/crca

//extern crate serde;
extern crate serde_json;

use std::io::{stdout, Write};
use curl::easy::Easy;
use std::collections::BTreeMap;

fn main() {
    let mut res = Vec::new();
    let mut handle = Easy::new();
    handle.url("https://auth.docker.io/token?service=registry.docker.io&scope=repository:library/alpine:pull").unwrap();
    let mut transfer = handle.transfer();
    transfer.write_function(|data| {
        res.extend_from_slice(data);
        Ok(data.len())
    }).unwrap();
    transfer.perform().unwrap();

    let res_string = String::from_utf8_lossy(&res);
    let res_json: BTreeMap<String, String> = serde_json::from_str(&res_string).unwrap();
    let token = res_json.get("token").unwrap();

    println!("{}", token);
}
