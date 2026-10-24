-- AUTH-6, ADR-0130: a tenant-scoped, credential-free index beside the
-- pre-tenant console custody row, and a bounded ledger of revoked token and
-- session-family identifier digests. Custody keeps no tenant or subject. The
-- index never answers authentication; deleting custody removes its index row.
CREATE TABLE public.console_session_index (
 tenant_id uuid NOT NULL, id uuid NOT NULL, subject text NOT NULL, issuer text NOT NULL,
 custody_hash bytea NOT NULL, family_digest bytea,
 created_at timestamptz NOT NULL DEFAULT now(), expires_at timestamptz NOT NULL,
 PRIMARY KEY (tenant_id,id), UNIQUE (custody_hash),
 FOREIGN KEY (tenant_id) REFERENCES public.tenants(id),
 FOREIGN KEY (custody_hash) REFERENCES public.console_sessions(token_hash) ON DELETE CASCADE,
 CHECK (length(subject) BETWEEN 1 AND 255), CHECK (length(issuer) BETWEEN 1 AND 512),
 CHECK (octet_length(custody_hash)=32),
 CHECK (family_digest IS NULL OR octet_length(family_digest)=32),
 CHECK (expires_at > created_at)
);
CREATE INDEX console_session_index_subject
 ON public.console_session_index(tenant_id,subject,created_at DESC,id DESC);
ALTER TABLE public.console_session_index ENABLE ROW LEVEL SECURITY;
ALTER TABLE public.console_session_index FORCE ROW LEVEL SECURITY;
CREATE POLICY console_session_index_tenant ON public.console_session_index
 USING (tenant_id=public.synveda_current_tenant()) WITH CHECK (tenant_id=public.synveda_current_tenant());
GRANT SELECT, INSERT, DELETE ON public.console_session_index TO synveda_app;

-- Revocation evidence is a digest of tenant, exact issuer, identifier kind
-- and opaque value; the value itself is never stored. Database time bounds
-- every row to at most twelve hours after its revocation.
CREATE TABLE public.token_revocations (
 tenant_id uuid NOT NULL, digest bytea NOT NULL, kind text NOT NULL,
 revoked_at timestamptz NOT NULL DEFAULT now(), expires_at timestamptz NOT NULL,
 PRIMARY KEY (tenant_id,digest),
 FOREIGN KEY (tenant_id) REFERENCES public.tenants(id),
 CHECK (octet_length(digest)=32), CHECK (kind IN ('token','family')),
 CHECK (expires_at > revoked_at), CHECK (expires_at <= revoked_at + interval '12 hours')
);
CREATE INDEX token_revocations_expiry ON public.token_revocations(tenant_id,expires_at);
ALTER TABLE public.token_revocations ENABLE ROW LEVEL SECURITY;
ALTER TABLE public.token_revocations FORCE ROW LEVEL SECURITY;
CREATE POLICY token_revocations_tenant ON public.token_revocations
 USING (tenant_id=public.synveda_current_tenant()) WITH CHECK (tenant_id=public.synveda_current_tenant());
GRANT SELECT, INSERT, DELETE ON public.token_revocations TO synveda_app;
