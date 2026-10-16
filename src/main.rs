fn main() {
    if let Err(error) = ikura_rust_prototype::run() {
        eprintln!("Ikura failed: {error}");
        std::process::exit(1);
    }
}
