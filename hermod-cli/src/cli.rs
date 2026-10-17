//! Argument parsing and subcommand dispatch. Handlers are thin wrappers:
//! they parse flags, build a [`hermod_core::config::Config`], call the
//! matching `hermod_core` API, and render progress. Wired up in M4.6
//! (CLI alignment with the Go version).

/// Parse the command line and dispatch to a subcommand handler.
pub fn run() {
    // M4.6: clap parsing + dispatch to crate::tx / rx / serve / trust.
    eprintln!("hermod: CLI wiring lands in M4.6");
    std::process::exit(1);
}
