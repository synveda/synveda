# CPR-45 / ADR-0102: retain the exact official Caddy release while removing
# its inherited cap_net_bind_service file capability. Canonical containers
# bind either high development ports or use a private network-namespace
# unprivileged-port sysctl in reference mode, so the process needs no effective
# Linux capability.
FROM caddy:2.11.4-alpine@sha256:5f5c8640aae01df9654968d946d8f1a56c497f1dd5c5cda4cf95ab7c14d58648

RUN setcap -r /usr/bin/caddy \
 && test -z "$(getcap /usr/bin/caddy)"

USER 65532:65532
