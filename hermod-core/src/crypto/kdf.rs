//! Key derivation: session keys from the CPace output K, including the
//! WireGuard PSK `HKDF(K, salt=channel_id, info="hermod-wg-psk-v1")`
//! and the future post-quantum recovery seam (D6/S1). See DEVELOPMENT.md
//! §3.3 and M1.2.
