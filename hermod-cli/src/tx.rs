//! `hermod tx` handler: thin CLI wrapper over the core transfer API. Parses
//! flags, builds a config, calls the tx orchestration in
//! `hermod_core::transfer`, and renders progress via the progress/tty
//! modules. Wired up in M4.4/M4.6.
