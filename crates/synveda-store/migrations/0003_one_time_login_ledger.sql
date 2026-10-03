-- OPS-7 / ADR-0126: deployment-scoped pre-tenant OIDC and CLI handoff state.
-- This is an additive, transactional migration. The state and code selectors
-- are hashes of random secrets; payloads must be deployment-key sealed by the
-- gateway before insertion. These tables precede tenant identification, like
-- console_sessions, and therefore cannot use tenant RLS.

CREATE TABLE public.pending_logins (
    state_hash bytea PRIMARY KEY,
    correlation_hash bytea,
    payload_sealed bytea NOT NULL,
    created_at timestamptz NOT NULL DEFAULT now(),
    expires_at timestamptz NOT NULL DEFAULT (now() + interval '10 minutes'),
    CONSTRAINT pending_logins_state_hash_check CHECK (octet_length(state_hash) = 32),
    CONSTRAINT pending_logins_correlation_hash_check CHECK (
        correlation_hash IS NULL OR octet_length(correlation_hash) = 32
    ),
    CONSTRAINT pending_logins_payload_check CHECK (
        octet_length(payload_sealed) BETWEEN 51 AND 8192
    ),
    CONSTRAINT pending_logins_expiry_check CHECK (expires_at > created_at)
);

CREATE INDEX pending_logins_expiry ON public.pending_logins (expires_at);
GRANT SELECT, INSERT, DELETE ON TABLE public.pending_logins TO synveda_app;

CREATE TABLE public.cli_handoffs (
    code_hash bytea PRIMARY KEY,
    state_hash bytea NOT NULL,
    payload_sealed bytea NOT NULL,
    created_at timestamptz NOT NULL DEFAULT now(),
    expires_at timestamptz NOT NULL DEFAULT (now() + interval '60 seconds'),
    CONSTRAINT cli_handoffs_code_hash_check CHECK (octet_length(code_hash) = 32),
    CONSTRAINT cli_handoffs_state_hash_check CHECK (octet_length(state_hash) = 32),
    CONSTRAINT cli_handoffs_payload_check CHECK (
        octet_length(payload_sealed) BETWEEN 51 AND 131072
    ),
    CONSTRAINT cli_handoffs_expiry_check CHECK (expires_at > created_at)
);

CREATE INDEX cli_handoffs_expiry ON public.cli_handoffs (expires_at);
GRANT SELECT, INSERT, DELETE ON TABLE public.cli_handoffs TO synveda_app;
