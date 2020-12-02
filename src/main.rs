// orca : CLI Container Runtime
// This program is created by nomlab in Okayama University
// author nomlab <https://github.com/nomlab>
//        miyake13000 <https://github.com/miyake13000/crca>

#[macro_use]
extern crate clap;
use clap::{App, Arg, SubCommand};

fn main() {
    let input = cli();
    let path = formatter(&input);
    println!("command : {}", path);
}

fn cli() -> App<'static, 'static> {

    let app = App::new(crate_name!())
        .version(crate_version!())
        .author(crate_authors!())
        .about(crate_description!())
        .arg(Arg::with_name("command")
            .help("command to execute in conainer")
            .required(true)
        )
        .arg(Arg::with_name("flg")
            .help("test flag")
            .short("f")
            .long("flag")
        )
        .arg(Arg::with_name("opt")
            .help("test option")
            .short("o")
            .long("option")
            .takes_value(true)
        )
        .subcommand(SubCommand::with_name("sub")
            .about("test suncommand")
            .arg(Arg::with_name("subflg")
                .help("test subcommand flag")
                .short("f")
                .long("flag")
            )
        );
    return app
}

fn formatter<'a>(input: &'a App) -> &'a str {
    let matches = input.get_matches();

    if let Some(o) = matches.value_of("command") {
        o
    }else {
        ""
    }
}
