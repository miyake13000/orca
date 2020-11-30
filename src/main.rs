// orca : CLI Container Runtime
// This program is created by nomlab in Okayama University
// author nomlab <https://github.com/nomlab>
//        miyake13000 <https://github.com/miyake13000/crca>

#[macro_use]
extern crate clap;
use clap::{App, Arg, SubCommand};

fn main() {
    let input = cli();
    parse_input(input);
}

fn cli() -> App<'static, 'static> {
    let app = App::new(crate_name!())
        .version(crate_version!())
        .author(crate_authors!())
        .about(crate_description!())
        .arg(Arg::with_name("pa")
            .help("positional argument")
            .required(true)
        )
        .arg(Arg::with_name("flg")
            .help("flag")
            .short("f")
            .long("flag")
        )
        .arg(Arg::with_name("opt")
            .help("option")
            .short("o")
            .long("option")
            .takes_value(true)
        )
        .subcommand(SubCommand::with_name("sub")
            .about("suncommand")
            .arg(Arg::with_name("subflg")
                .help("sub flag")
                .short("f")
                .long("flag")
            )
        );
    return app
}

fn parse_input(input :App){
    let matches = input.get_matches();

    if let Some(o) = matches.value_of("pa") {
        println!("Value for pa: {}", o);
    }
    if let Some(o) = matches.value_of("opt") {
        println!("Value for opt: {}", o);
    }
    println!("flg is {}", if matches.is_present("flg") {"true"} else {"false"});
    if let Some(ref matches) = matches.subcommand_matches("sub") {
        println!("uysed sub");
        println!("subflg is {}", if matches.is_present("subflg") {"true"} else {"false"});
    }
}
