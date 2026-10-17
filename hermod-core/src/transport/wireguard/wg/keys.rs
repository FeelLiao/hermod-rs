//! Per-session ephemeral X25519 keys and PSK derivation
//! (`HKDF(K, salt=channel_id, info="hermod-wg-psk-v1")`). Keys are never
//! reused and never written to disk. See DEVELOPMENT.md §3.3 and M3.1.
