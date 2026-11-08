//! Command-line front end of Memora Studio (see `memora_desktop_lib::cli`). Console program, no window.

fn main() {
    std::process::exit(memora_desktop_lib::cli::run(std::env::args().skip(1).collect()));
}
