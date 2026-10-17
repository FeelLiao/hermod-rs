//! Hermod CLI entry point — a thin shell over `hermod_core` (decision D17).

mod cli;
mod progress;
mod rx;
mod serve;
mod trust;
mod tty;
mod tx;
mod verbosity;

fn main() {
    cli::run();
}
