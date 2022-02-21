// orca : CLI Container management tool
// This program is managemented by nomlab <https://github.com/nomlab>

use anyhow::Result;
use dirs::home_dir;
use orca::args::Args;
use orca::container::Container;
use orca::image::Image;

fn main() -> Result<()> {
    let default_image_name = String::from("debian");
    let default_image_tag = String::from("latest");
    let default_container_name = String::from("orca");
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

    let image_root = home_dir().unwrap().join(".local").join("orca");

    let image = Image::new(image_root, image_name, image_tag, container_name)
        .workdir("/tmp/orca/image")
        .display_progress(true);

    if args.init_flag && image.exists_container() {
        print!("Removing old image");
        image.remove_container()?;
    }
    if !image.exists_image() {
        image.download()?;
    }
    if !image.exists_container() {
        println!("Creating container");
        image.create_container_image()?;
    } else {
        println!("Enter image already exists")
    }

    let mut working_container = Container::new(image, command, args.cmd_args, args.netns_flag)?;
    working_container.connect_tty()?;

    let used_image = working_container.wait()?;

    if args.remove_flag {
        used_image.remove_container()?;
    }

    Ok(())
}
