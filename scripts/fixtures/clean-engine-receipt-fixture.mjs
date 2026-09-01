const REGISTRY_IMAGE =
  "registry:3.1.1@sha256:1be55279f18a2fe1a74edf2664cac61c1bea305b7b4642dab412e7affdcb3e33";

export function cleanEngineReceiptResult(phase, fixtureId) {
  switch (phase) {
    case "provider-create-intent":
      return {
        cleanup_command: "colima-delete-data-force",
        preexisting_resource: "absent",
        provider_contract_sha256: "2".repeat(64),
        provider_resource: `synveda-cpr45-${fixtureId}`,
        provider_root_key: `sv-c45-${fixtureId.slice(0, 16)}`,
      };
    case "provider-create-passed":
      return {
        colima_version: "0.10.3",
        docker_client_version: "29.4.0",
        docker_server_version: "29.4.0",
        engine_identity_sha256: "3".repeat(64),
        initial_containers: 0,
        initial_images: 0,
        initial_networks: ["bridge", "host", "none"],
        initial_volumes: 0,
        platform: "darwin-arm64-colima-vz",
        socket_contract: "receipt-owned-unix",
      };
    case "registry-intent":
      return {
        authentication: "basic-bcrypt-cost-12",
        container: `synveda-cpr45-registry-${fixtureId.slice(0, 16)}`,
        image: REGISTRY_IMAGE,
        port: 54_321,
        transport: "tls-loopback",
      };
    case "registry-passed":
      return {
        authenticated_pull: true,
        authenticated_push: true,
        basic_challenge: true,
        canary_image_sha256: "4".repeat(64),
        certificate_sha256: "5".repeat(64),
        negative_status: 401,
        unauthenticated_pull_rejected: true,
        wrong_password_rejected: true,
      };
    case "proxy-intent":
      return {
        config: "synthetic-nonsecret-v1",
        expected_injected_variables: 10,
        expected_runtime_empty_variables: 10,
      };
    case "proxy-passed":
      return { auth_preserved: true, injected_variables: 10, runtime_empty_variables: 10 };
    case "builder-canary-intent":
      return {
        builder: `synveda-cpr45-canary-${fixtureId.slice(0, 16)}`,
        canonical_builder: "default",
        endpoint: "loopback-inert-tcp",
        expected_connections: 0,
      };
    case "builder-canary-passed":
      return {
        canonical_builder_driver: "docker",
        canonical_builder_endpoint: "default",
        connections: 0,
        private_buildx_removed: true,
      };
    case "compose-browser-intent":
      return {
        capture: "disabled",
        profiles: ["browser-acceptance", "demo"],
        project: `synveda-development-acceptance-${fixtureId.slice(0, 24)}`,
      };
    case "compose-browser-passed":
      return {
        admin_admitted: true,
        browser_exit: 0,
        captured_artifacts: 0,
        container_proxy_empty_variables: 10,
        logout: true,
        pkce_s256: true,
      };
    case "project-cleanup-intent":
      return {
        project: `synveda-development-acceptance-${fixtureId.slice(0, 24)}`,
        resolver: "managed-test-block",
        scope: "exact-receipt-owned-only",
      };
    case "project-cleanup-passed":
      return {
        builder_canary_absent: true,
        project_absent: true,
        registry_absent: true,
        resolver_absent: true,
        runtime_secrets_absent: true,
      };
    case "provider-cleanup-intent":
      return {
        command: "colima-delete-data-force",
        provider_resource: `synveda-cpr45-${fixtureId}`,
        scope: "exact-receipt-owned-only",
      };
    case "provider-cleanup-passed":
      return {
        context_absent: true,
        inert_staging_absent: true,
        provider_absent: true,
        runtime_root_absent: true,
        socket_absent: true,
        source_closure_unchanged: true,
      };
    default:
      throw new Error(`missing receipt fixture result for ${phase}`);
  }
}
