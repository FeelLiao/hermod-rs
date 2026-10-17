//! `hermod rx` handler: thin CLI wrapper over the core transfer API. Parses
//! flags, builds a config, calls the rx orchestration in
//! `hermod_core::transfer`, and renders progress. Wired up in M4.4/M4.6.
