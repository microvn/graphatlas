//! Middleware module — security gate enforced on every protected route.
//!
//! Phase 1 invariant (Spec A C-cross-2 + A-C11):
//! For any non-GET request OR any non-`/api/health` request, ALL THREE
//! checks must pass:
//!   1. `Origin` header ∈ allowed_origins  → else 403 bad_origin
//!   2. `Host`   header ∈ allowed_hosts    → else 421 misdirected_request
//!      (defends against DNS rebinding — attacker resolves their domain
//!      to 127.0.0.1 but Host stays `attacker.com`)
//!   3. `X-GA-Token` header == cfg.token   → else 403 bad_token
//!
//! Defense-in-depth note: even though we bind 127.0.0.1, any browser tab
//! on the same machine can hit our port via cross-origin fetch. The
//! Origin allowlist + custom token header (which forces a preflight on
//! cross-origin requests) closes that gap.

pub mod security;
