use clap::{crate_authors, crate_description, crate_name, crate_version};
use clap::{App, AppSettings, Arg};

pub struct Args {
    pub image_name: Option<String>,
    pub image_tag: Option<String>,
    pub container_name: Option<String>,
    pub command: Option<String>,
    pub cmd_args: Option<Vec<String>>,
    pub init_flag: bool,
    pub remove_flag: bool,
    pub netns_flag: bool,
}

impl Args {
    pub fn new() -> Args {
        Args {
            image_name: None,
            image_tag: None,
            container_name: None,
            command: None,
            cmd_args: None,
            init_flag: false,
            remove_flag: false,
            netns_flag: false,
        }
    }

    pub fn set_args(&mut self) {
        let app = App::new(crate_name!())
            .setting(AppSettings::AllowExternalSubcommands)
            .version(crate_version!())
            .author(crate_authors!())
            .about(crate_description!())
            .usage("orca [FLAGS] [OPTIONS] [COMMAND [ARGS..]]")
            .arg(
                Arg::with_name("image")
                    .short("i")
                    .long("image")
                    .help("Image name of container image")
                    .takes_value(true),
            )
            .arg(
                Arg::with_name("tag")
                    .short("t")
                    .long("tag")
                    .help("Tag name of container iamge")
                    .takes_value(true),
            )
            .arg(
                Arg::with_name("name")
                    .short("n")
                    .long("name")
                    .help("Name of container")
                    .takes_value(true),
            )
            .arg(
                Arg::with_name("init")
                    .long("init")
                    .help("Initialize contaier image before running a command"),
            )
            .arg(
                Arg::with_name("remove")
                    .long("rm")
                    .help("Remove container image after running a command"),
            )
            .arg(
                Arg::with_name("use_netns")
                    .long("use-netns")
                    .help("Isolate network namespace"),
            );

        let matches = app.get_matches();

        if let Some(o) = matches.value_of("image") {
            self.image_name = Some(o.to_string());
        }
        if let Some(o) = matches.value_of("tag") {
            self.image_tag = Some(o.to_string());
        }
        if let Some(o) = matches.value_of("name") {
            self.container_name = Some(o.to_string());
        }
        self.init_flag = matches.is_present("init");
        self.remove_flag = matches.is_present("remove");
        self.netns_flag = matches.is_present("use_netns");
        match matches.subcommand() {
            (external, Some(arg_matches)) => {
                let command = if external.is_empty() {
                    None
                } else {
                    Some(external.to_string())
                };
                let args = if let Some(values) = arg_matches.values_of("") {
                    Some(values.map(|arg| arg.to_string()).collect())
                } else {
                    None
                };
                self.command = command;
                self.cmd_args = args;
            }
            (external, None) => {
                let command = if external.is_empty() {
                    None
                } else {
                    Some(external.to_string())
                };
                self.command = command;
            }
        };
    }
}
