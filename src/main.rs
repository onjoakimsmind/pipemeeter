fn main() {
    if let Err(error) = pipemeeter::run() {
        eprintln!("pipemeeter failed: {error}");
        std::process::exit(1);
    }
}
