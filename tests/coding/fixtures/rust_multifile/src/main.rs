fn main() {
    let input = std::env::args().nth(1).unwrap_or_default();
    match fixture_rust_multifile::parser::parse_label(&input) {
        Ok(label) => println!("{}", label.0),
        Err(error) => { eprintln!("{error}"); std::process::exit(2); }
    }
}
