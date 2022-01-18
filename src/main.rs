// orca : CLI Container management tool
// This program is managemented by nomlab <https://github.com/nomlab>

use anyhow::{Context, Result};
use dirs::home_dir;
use orca::args::Args;
use orca::command::Command;
use orca::container::Container;
use orca::image::Image;
use orca::terminal::Terminal;

fn main() -> Result<()> {
    let default_image_name = String::from("debian");
    let default_image_tag = String::from("latest");
    let default_container_name = String::from("_default");
    let default_command = String::from("sh");

    let mut args = Args::new();
    args.set_args(); // Set args from stdin into args instanse

    let image_name = if let Some(image_name) = args.image_name {
        image_name
    } else {
        default_image_name
    };
    let image_tag = if let Some(image_tag) = args.image_tag {
        image_tag
    } else {
        default_image_tag
    };
    let container_name = if let Some(container_name) = args.container_name {
        container_name
    } else {
        default_container_name
    };
    let command = if let Some(command) = args.command {
        command
    } else {
        default_command
    };

    let image_path_prefix = format!(
        "{}/.local/orca",
        home_dir()
            .unwrap()
            .to_str()
            .context("Failed get HOME from environment variable")?,
    );

    let image = Image::new(image_path_prefix, image_name, image_tag, container_name);

    if args.init_flag {
        println!("Remove image");
        image.remove()?;
    }
    if !image.image_exists() {
        println!("Download image");
        image.download()?;
    }
    if !image.container_exists() {
        println!("Extract image");
        image.extract()?;
    }

    let mut working_container = Container::new(image, command, args.cmd_args, args.netns_flag)?;

    if Command::new("newuidmap", Option::<Vec<String>>::None).is_exist() {
        working_container.map_id_with_subuid()?;
    } else {
        working_container.map_id()?;
    }

    working_container.connect_tty()?;

    let mut terminal = Terminal::new()?;
    terminal.into_raw_mode()?;

    let used_image = working_container.wait()?;

    if args.remove_flag {
        used_image.remove()?;
    }

    Ok(())
}
