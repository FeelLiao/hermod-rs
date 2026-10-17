//! In-tunnel HTTP server (tx side): serves the file with `Content-Length` or
//! chunked encoding, computes SHA-256 while sending, and answers `POST /done`
//! with the digest verdict. See DEVELOPMENT.md §3.4/§3.6 and M4.3.
