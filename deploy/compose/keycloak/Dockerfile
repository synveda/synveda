# CPR-45 / ADR-0102: the bundled provider is an optimized production-mode
# Keycloak build. The official multi-architecture index is pinned exactly.
FROM quay.io/keycloak/keycloak:26.7.2@sha256:9d1f1b2b7261ff53c66cb1092dfcdc34a5fb77e81f9e6a6e75b8b6a795de8067 AS builder

ENV KC_DB=postgres \
    KC_FEATURES_DISABLED=identity-brokering-api,twitter-broker \
    KC_HEALTH_ENABLED=true \
    KC_METRICS_ENABLED=true
RUN /opt/keycloak/bin/kc.sh build

FROM quay.io/keycloak/keycloak:26.7.2@sha256:9d1f1b2b7261ff53c66cb1092dfcdc34a5fb77e81f9e6a6e75b8b6a795de8067
COPY --from=builder /opt/keycloak/ /opt/keycloak/
COPY --chmod=0755 deploy/compose/keycloak/keycloak-entrypoint /opt/keycloak/bin/keycloak-entrypoint

USER 1000:1000
EXPOSE 8080 9000
ENTRYPOINT ["/opt/keycloak/bin/keycloak-entrypoint"]
CMD ["start", "--optimized"]
