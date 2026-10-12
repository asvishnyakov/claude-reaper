mod cli;
mod reaper;
mod sessions;
mod transcript;

fn main() -> std::process::ExitCode {
    cli::main()
}
