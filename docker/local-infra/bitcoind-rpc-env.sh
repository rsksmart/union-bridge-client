# Single source of the local Bitcoin RPC credentials. Source (don't execute) this to populate
# BITCOIND_USER / BITCOIND_PASSWORD from BITCOIND_URL — the same variable BitVMX uses (via
# scripts/setup-operators.sh) — so the credentials are declared once, in your .envrc.
#
# Already-set values win (explicit override). When BITCOIND_URL is unset it falls back to the bundled
# regtest defaults, so the scripts still work without .envrc loaded (CI, direct invocation).
#
# Only user:password are read from BITCOIND_URL — NOT the host or port. Its host is the
# container-reachable one (host.docker.internal) that BitVMX needs, while host-side bitcoin-cli talks
# to 127.0.0.1:18443 (the regtest default). BitVMX and the wallet CLI honor a non-default host/port
# via their own full URLs (BITCOIND_URL / WALLET_RPC_URL), so there's nothing to derive here.

_bre_user="foo"
_bre_pass="rpcpassword"
if [[ -n "${BITCOIND_URL:-}" ]]; then
  _bre_rest="${BITCOIND_URL#*://}"
  if [[ "$_bre_rest" == *@* ]]; then
    _bre_creds="${_bre_rest%%@*}"
    _bre_user="${_bre_creds%%:*}"
    # Only override the password when the userinfo actually carries "user:pass". A bare "user@host"
    # keeps the default password instead of reusing the username.
    if [[ "$_bre_creds" == *:* ]]; then
      _bre_pass="${_bre_creds#*:}"
    fi
  fi
fi

export BITCOIND_USER="${BITCOIND_USER:-$_bre_user}"
export BITCOIND_PASSWORD="${BITCOIND_PASSWORD:-$_bre_pass}"

unset _bre_user _bre_pass _bre_rest _bre_creds 2>/dev/null || true
