//! In-tunnel HTTP client (rx side): a single GET pulls the whole file, then
//! `POST /done` submits the digest verdict. See DEVELOPMENT.md §3.4/§3.6 and
//! M4.3.
