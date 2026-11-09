fn main() {
    if let Err(error) = wallapop_mcp::cli::run(std::env::args().skip(1).collect()) {
        eprintln!("wallapop-mcp: {error}");
        if let Some(partial) = error.partial {
            eprintln!("{partial}");
        }
        std::process::exit(1);
    }
}
