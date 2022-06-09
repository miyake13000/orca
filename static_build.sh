#!/bin/bash

cd "$(dirname $0)"
docker run --rm -it -v "$(pwd)":/home/rust/src ekidd/rust-musl-builder cargo build --target=x86_64-unknown-linux-musl --release
