//! Progress rendering: determinate when `Content-Length` is known,
//! indeterminate otherwise (matches Go behavior). The core library exposes
//! progress through a trait so non-CLI clients can plug their own renderer.
//! See DEVELOPMENT.md §3.6.
