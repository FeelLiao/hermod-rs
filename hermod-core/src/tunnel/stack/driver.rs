//! smoltcp interface polling and socket management on the data thread
//! (blocking poll + wakeup fd + 100 ms timer tick for WireGuard timers).
//! See DEVELOPMENT.md §3.8 and M4.1.
