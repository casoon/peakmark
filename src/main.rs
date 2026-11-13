//! The `peakmark` command: a thin shell around [`peakmark::cli`].

fn main() -> std::process::ExitCode {
    peakmark::cli::main(std::env::args().skip(1))
}
