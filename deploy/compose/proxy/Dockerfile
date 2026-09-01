# CPR-45 / ADR-0102: retain the exact official Caddy release while removing
# its inherited cap_net_bind_service file capability. Canonical containers
# bind either high development ports or use a private network-namespace
# unprivileged-port sysctl in reference mode, so the process needs no effective
# Linux capability.
FROM caddy:2.11.4-alpine@sha256:5f5c8640aae01df9654968d946d8f1a56c497f1dd5c5cda4cf95ab7c14d58648
ARG HTTP_PROXY
ARG http_proxy
ARG HTTPS_PROXY
ARG https_proxy
ARG NO_PROXY
ARG no_proxy
ARG FTP_PROXY
ARG ftp_proxy
ARG ALL_PROXY
ARG all_proxy
COPY --chmod=0555 deploy/compose/scripts/assert-build-proxy-closed /usr/local/bin/assert-build-proxy-closed
RUN /usr/local/bin/assert-build-proxy-closed

RUN setcap -r /usr/bin/caddy \
 && test -z "$(getcap /usr/bin/caddy)"

USER 65532:65532
