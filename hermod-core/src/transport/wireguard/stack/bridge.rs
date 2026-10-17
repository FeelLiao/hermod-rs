//! Bounded-channel bridge that turns smoltcp sockets into
//! `AsyncRead + AsyncWrite` with real backpressure and no busy-waiting.
//! See DEVELOPMENT.md §3.4/§3.8 and M4.2.
