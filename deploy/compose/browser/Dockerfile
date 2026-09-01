# One-shot CPR-45 browser acceptance. This image is fixture tooling, not a
# product service. The official multi-architecture image and matching npm
# package are pinned to Playwright 1.62.1.
FROM mcr.microsoft.com/playwright:v1.62.1-noble@sha256:dcc5531e97840b9b5e794f2814476b21571c5124a3fca2267d73041f56e7580e
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

WORKDIR /workspace
RUN corepack enable
COPY package.json pnpm-lock.yaml pnpm-workspace.yaml ./
COPY deploy/compose/browser/package.json deploy/compose/browser/package.json
RUN pnpm install --frozen-lockfile --prod --filter @synveda/compose-browser-acceptance...
COPY --chmod=0555 deploy/compose/browser/console-login.mjs deploy/compose/browser/console-login.mjs
COPY --chmod=0444 deploy/compose/browser/console-login-runner.mjs deploy/compose/browser/console-login-runner.mjs
COPY --chmod=0444 deploy/compose/browser/console-login-contract.mjs deploy/compose/browser/console-login-contract.mjs
COPY --chmod=0444 deploy/compose/browser/PLAYWRIGHT-LICENSE deploy/compose/browser/PLAYWRIGHT-NOTICE deploy/compose/browser/seccomp_profile.NOTICE /usr/share/licenses/synveda-browser-acceptance/

ENV HOME=/tmp/browser-home \
    PLAYWRIGHT_BROWSERS_PATH=/ms-playwright
USER 65532:65532
WORKDIR /workspace/deploy/compose/browser
ENTRYPOINT ["node", "console-login.mjs"]
