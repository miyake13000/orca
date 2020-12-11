// This program is created by nomlab in Okayama University
// https://github.com/miyake13000/crca

use std::io::{stdout, Write};
use curl::easy::Easy;

fn main() {
    let mut easy = Easy::new();
    easy.url("https://auth.docker.io/token?service=registry.docker.io&scope=repository:library/alpine:pull").unwrap();
    easy.write_function(|data| {
        stdout().write_all(data).unwrap();
        Ok(data.len())
    }).unwrap();
    easy.perform().unwrap();

    println!("{}",easy.response_code().unwrap());
}
