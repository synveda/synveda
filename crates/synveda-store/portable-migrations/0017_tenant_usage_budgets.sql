-- Request limits and tenant usage budgets, ADR-0159: durable per-tenant daily
-- usage and expiring budget overrides. The usage day is the database's UTC
-- date. The runtime role may create a day's row and add to it but never
-- delete one, so a budget change cannot erase usage evidence. An override can
-- only narrow the deployment ceiling at charge time and always expires.
CREATE TABLE public.tenant_usage (
    tenant_id uuid NOT NULL,
    meter text NOT NULL,
    usage_day date NOT NULL,
    units bigint NOT NULL,
    PRIMARY KEY (tenant_id, meter, usage_day),
    FOREIGN KEY (tenant_id) REFERENCES public.tenants(id),
    CONSTRAINT tenant_usage_meter_check
        CHECK (meter IN ('session_event_bytes', 'context_runs')),
    CONSTRAINT tenant_usage_units_check CHECK (units >= 0)
);
ALTER TABLE public.tenant_usage ENABLE ROW LEVEL SECURITY;
ALTER TABLE public.tenant_usage FORCE ROW LEVEL SECURITY;
CREATE POLICY tenant_usage_tenant ON public.tenant_usage
    USING (tenant_id = public.synveda_current_tenant())
    WITH CHECK (tenant_id = public.synveda_current_tenant());
GRANT SELECT, INSERT ON TABLE public.tenant_usage TO synveda_app;
GRANT UPDATE (units) ON TABLE public.tenant_usage TO synveda_app;

CREATE TABLE public.tenant_usage_budgets (
    tenant_id uuid NOT NULL,
    meter text NOT NULL,
    daily_limit bigint NOT NULL,
    reason text NOT NULL,
    set_by text NOT NULL,
    set_at timestamp with time zone NOT NULL,
    expires_at timestamp with time zone NOT NULL,
    PRIMARY KEY (tenant_id, meter),
    FOREIGN KEY (tenant_id) REFERENCES public.tenants(id),
    CONSTRAINT tenant_usage_budgets_meter_check
        CHECK (meter IN ('session_event_bytes', 'context_runs')),
    CONSTRAINT tenant_usage_budgets_limit_check CHECK (daily_limit >= 0),
    CONSTRAINT tenant_usage_budgets_reason_check
        CHECK (btrim(reason) <> '' AND char_length(reason) <= 500),
    CONSTRAINT tenant_usage_budgets_actor_check
        CHECK (btrim(set_by) <> '' AND char_length(set_by) <= 255),
    CONSTRAINT tenant_usage_budgets_expiry_check
        CHECK (expires_at > set_at AND expires_at <= set_at + interval '90 days')
);
ALTER TABLE public.tenant_usage_budgets ENABLE ROW LEVEL SECURITY;
ALTER TABLE public.tenant_usage_budgets FORCE ROW LEVEL SECURITY;
CREATE POLICY tenant_usage_budgets_tenant ON public.tenant_usage_budgets
    USING (tenant_id = public.synveda_current_tenant())
    WITH CHECK (tenant_id = public.synveda_current_tenant());
GRANT SELECT, INSERT, DELETE ON TABLE public.tenant_usage_budgets TO synveda_app;
GRANT UPDATE (daily_limit, reason, set_by, set_at, expires_at)
    ON TABLE public.tenant_usage_budgets TO synveda_app;
