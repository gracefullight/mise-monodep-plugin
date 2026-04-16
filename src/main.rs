mod cli;
mod dedup;
mod engine;
mod manifest;
mod models;
mod pm;

fn main() {
    let exit_code = cli::run();
    std::process::exit(exit_code);
}
