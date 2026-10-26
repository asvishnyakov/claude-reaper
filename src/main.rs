mod cli;
mod process;
mod reaper;
mod sessions;
mod stop;
mod transcript;

fn main() -> std::process::ExitCode {
    cli::main()
}
