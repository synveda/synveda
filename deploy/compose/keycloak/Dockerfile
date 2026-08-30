# Build the shared mounted-input helper as a small dynamically linked binary.
# The native-architecture build is executed again in the final UBI-based image
# so a glibc/loader mismatch fails the image build; glibc is not statically
# embedded in the distributed helper.
FROM rust:1.96.0-bookworm@sha256:5e2214abe154fe26e39f64488952e5c991eeed1d6d6da7cc8381ae83927f0cfc AS snapshot-builder
COPY deploy/compose/postgres/synveda-input-snapshot.c /tmp/synveda-input-snapshot.c
RUN cc -std=c11 -O2 -Wall -Wextra -Werror \
       /tmp/synveda-input-snapshot.c -o /opt/synveda-input-snapshot \
    && strip /opt/synveda-input-snapshot \
    && chmod 0555 /opt/synveda-input-snapshot

# CPR-45 / ADR-0102: the bundled provider is an optimized production-mode
# Keycloak build. The official multi-architecture index is pinned exactly.
FROM quay.io/keycloak/keycloak:26.7.2@sha256:9d1f1b2b7261ff53c66cb1092dfcdc34a5fb77e81f9e6a6e75b8b6a795de8067 AS builder

ENV KC_DB=postgres \
    KC_FEATURES_DISABLED=identity-brokering-api,twitter-broker \
    KC_HEALTH_ENABLED=true \
    KC_METRICS_ENABLED=true
COPY deploy/compose/keycloak/SynvedaKeycloakProjection.java /tmp/SynvedaKeycloakProjection.java
COPY deploy/compose/keycloak/SynvedaKeycloakProjectionSelfTest.java /tmp/SynvedaKeycloakProjectionSelfTest.java
COPY --chmod=0555 deploy/compose/keycloak/synveda-projection-self-test /tmp/synveda-projection-self-test
COPY --chmod=0444 deploy/compose/keycloak/synveda-authority-stage /tmp/synveda-authority-stage
COPY --chmod=0555 deploy/compose/keycloak/synveda-authority-stage-self-test /tmp/synveda-authority-stage-self-test
RUN /usr/bin/java --add-modules jdk.compiler \
      -cp '/opt/keycloak/lib/lib/main/*' com.sun.tools.javac.Main \
      -cp '/opt/keycloak/lib/lib/main/com.fasterxml.jackson.core.jackson-annotations-2.21.jar:/opt/keycloak/lib/lib/main/com.fasterxml.jackson.core.jackson-core-2.21.5.jar:/opt/keycloak/lib/lib/main/com.fasterxml.jackson.core.jackson-databind-2.21.5.jar' \
      -d /opt/keycloak/bin /tmp/SynvedaKeycloakProjection.java \
	    && test -r /opt/keycloak/bin/SynvedaKeycloakProjection.class
RUN mkdir -p /tmp/synveda-projection-test \
    && /usr/bin/java --add-modules jdk.compiler \
      -cp '/opt/keycloak/lib/lib/main/*' com.sun.tools.javac.Main \
      -cp '/opt/keycloak/bin:/opt/keycloak/lib/lib/main/com.fasterxml.jackson.core.jackson-annotations-2.21.jar:/opt/keycloak/lib/lib/main/com.fasterxml.jackson.core.jackson-core-2.21.5.jar:/opt/keycloak/lib/lib/main/com.fasterxml.jackson.core.jackson-databind-2.21.5.jar' \
      -d /tmp/synveda-projection-test /tmp/SynvedaKeycloakProjectionSelfTest.java \
    && /usr/bin/java \
      -cp '/tmp/synveda-projection-test:/opt/keycloak/bin:/opt/keycloak/lib/lib/main/*' \
      SynvedaKeycloakProjectionSelfTest
RUN /tmp/synveda-projection-self-test
RUN /tmp/synveda-authority-stage-self-test
RUN /opt/keycloak/bin/kc.sh build

FROM quay.io/keycloak/keycloak:26.7.2@sha256:9d1f1b2b7261ff53c66cb1092dfcdc34a5fb77e81f9e6a6e75b8b6a795de8067
# KC-SERVICES0077 includes the mounted bootstrap identifier at INFO. Retain
# service warnings/errors without allowing that input into default logs.
ENV KC_LOG_LEVEL_ORG_KEYCLOAK_SERVICES=warn
COPY --from=builder /opt/keycloak/ /opt/keycloak/
COPY --from=snapshot-builder --chmod=0555 /opt/synveda-input-snapshot /opt/keycloak/bin/synveda-input-snapshot
COPY --chmod=0755 deploy/compose/keycloak/keycloak-entrypoint /opt/keycloak/bin/keycloak-entrypoint
COPY --chmod=0555 deploy/compose/keycloak/synveda-generation-gate /opt/keycloak/bin/synveda-generation-gate
COPY --chmod=0555 deploy/compose/keycloak/synveda-generation-gate-self-test /opt/keycloak/bin/synveda-generation-gate-self-test
COPY --chmod=0555 deploy/compose/keycloak/synveda-keycloak-health /opt/keycloak/bin/synveda-keycloak-health
COPY --chmod=0444 deploy/compose/keycloak/synveda-authority-stage /opt/keycloak/bin/synveda-authority-stage
COPY --chmod=0555 deploy/compose/keycloak/synveda-realm-converge /opt/keycloak/bin/synveda-realm-converge
COPY --chmod=0555 deploy/compose/keycloak/synveda-realm-supervise /opt/keycloak/bin/synveda-realm-supervise
COPY --chmod=0444 deploy/compose/keycloak/synveda-groups-mapper.json /opt/keycloak/conf/synveda-groups-mapper.json
COPY --chmod=0444 deploy/compose/keycloak/synveda-audience-mapper.json /opt/keycloak/conf/synveda-audience-mapper.json
RUN test -x /usr/bin/timeout
RUN /opt/keycloak/bin/synveda-generation-gate-self-test
RUN /opt/keycloak/bin/synveda-keycloak-health self-test
RUN printf 'snapshot-probe\n' > /tmp/synveda-snapshot-input \
    && /opt/keycloak/bin/synveda-input-snapshot \
       /tmp/synveda-snapshot-input /tmp/synveda-snapshot-output \
    && test -s /tmp/synveda-snapshot-output \
    && rm -f /tmp/synveda-snapshot-input /tmp/synveda-snapshot-output

USER 1000:1000
EXPOSE 8080 9000
ENTRYPOINT ["/opt/keycloak/bin/keycloak-entrypoint"]
CMD ["start", "--optimized"]
