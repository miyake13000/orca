pub trait Exit<T, E> {
    fn or_exit(self, mesg: &str) -> T;
}

impl<T, E> Exit<T, E> for std::result::Result<T, ()> {
    fn or_exit(self, mesg: &str)  -> T {
        match self {
            Ok(res) => res,
            Err(_) => {
                eprintln!("{}", mesg);
                std::process::exit(1);
            }
        }
    }
}
