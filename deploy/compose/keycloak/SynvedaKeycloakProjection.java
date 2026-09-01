// CPR-45: exact, content-free readback of the Keycloak objects managed by
// synveda-realm-converge. Keycloak already ships Jackson; this helper adds no
// provider, package or network dependency to the optimized image.

import com.fasterxml.jackson.core.JsonFactory;
import com.fasterxml.jackson.core.StreamReadFeature;
import com.fasterxml.jackson.databind.DeserializationFeature;
import com.fasterxml.jackson.databind.JsonNode;
import com.fasterxml.jackson.databind.ObjectMapper;
import java.nio.file.Files;
import java.nio.file.Path;
import java.net.URI;
import java.net.URLEncoder;
import java.net.http.HttpClient;
import java.net.http.HttpRequest;
import java.net.http.HttpResponse;
import java.io.ByteArrayOutputStream;
import java.math.BigInteger;
import java.nio.ByteBuffer;
import java.nio.charset.StandardCharsets;
import java.security.KeyFactory;
import java.security.MessageDigest;
import java.security.Signature;
import java.security.spec.RSAPublicKeySpec;
import java.time.Duration;
import java.time.Instant;
import java.util.ArrayList;
import java.util.Base64;
import java.util.HashMap;
import java.util.HashSet;
import java.util.List;
import java.util.Map;
import java.util.Set;
import java.util.TreeMap;
import java.util.concurrent.CompletableFuture;
import java.util.concurrent.ExecutionException;
import java.util.concurrent.Flow;
import java.util.concurrent.TimeUnit;
import java.util.concurrent.TimeoutException;
import java.util.regex.Pattern;

public final class SynvedaKeycloakProjection {
    private static final ObjectMapper JSON = strictJson();
    private static final List<String> OUTPUT = new ArrayList<>();
    private static final int MAX_INPUT_BYTES = 1_048_576;
    private static final int MAX_TOKEN_RESPONSE_BYTES = 65_536;
    private static final int MAX_TOKEN_BYTES = 131_072;
    private static final int MAX_DEMO_CLEANUP_ITEMS = 4;
    private static final Duration AUTHORITY_REQUEST_TIMEOUT = Duration.ofSeconds(2);
    private static final Duration AUTHORITY_PROOF_BUDGET = Duration.ofSeconds(34);
    private static final Duration AUTHORITY_CLEANUP_BUDGET = Duration.ofSeconds(6);
    private static final String ADMIN_CLIENT = "admin-cli";
    private static final String REFUSAL_MESSAGE =
        "keycloak-projection: input was refused";
    private static final String ADMIN_SERVER_URL = "http://keycloak:8080";
    private static final String ADMIN_REALM_URL =
        ADMIN_SERVER_URL + "/realms/master";
    private static final Pattern UUID = Pattern.compile(
        "[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}"
    );
    private static final Pattern KEYCLOAK_SESSION_STATE = Pattern.compile(
        "[A-Za-z0-9_-]{24}"
    );
    private static final Set<String> CONVERGENCE_ROLES = Set.of(
        "manage-clients", "manage-realm", "manage-users"
    );
    private static final Set<String> EFFECTIVE_AUDIT_ROLES = Set.of(
        "query-groups", "query-users", "view-users"
    );
    private static final String MANAGED_CONTRACT_KEY = "synvedaManagedContract";
    private static final String RETIREMENT_STATE_KEY = "synvedaBootstrapRetirementState";
    private static final String BOOTSTRAP_USER_ID_KEY = "synvedaBootstrapUserId";
    private static final String CONVERGENCE_USER_ID_KEY = "synvedaConvergenceUserId";
    private static final Set<String> MANAGED_ATTRIBUTE_KEYS = Set.of(
        MANAGED_CONTRACT_KEY,
        RETIREMENT_STATE_KEY,
        BOOTSTRAP_USER_ID_KEY,
        CONVERGENCE_USER_ID_KEY
    );

    enum AuthorityStage {
        TOKEN_HTTP(20, "token-http"),
        TOKEN_ENVELOPE(21, "token-envelope"),
        JWKS_HTTP(22, "jwks-http"),
        JWKS_SIGNATURE(23, "jwks-signature"),
        TOKEN_CLAIMS(24, "token-claims"),
        ACCESSIBLE_REALMS(25, "accessible-realms"),
        TARGET_REALM(26, "target-realm"),
        MASTER_INVENTORY(27, "master-inventory"),
        MASTER_SELF_QUERY(28, "master-self-query"),
        MASTER_SELF(29, "master-self"),
        MASTER_FEDERATED_IDENTITIES(30, "master-federated-identities"),
        MASTER_CREDENTIALS(31, "master-credentials"),
        MASTER_CLIENTS(32, "master-clients"),
        MASTER_SESSION_STATS(33, "master-session-stats"),
        DENY_CREATE_REALM(34, "deny-create-realm"),
        DENY_CREATE_MASTER_USER(35, "deny-create-master-user"),
        DENY_UPDATE_MASTER_SELF(36, "deny-update-master-self"),
        DENY_ADD_MASTER_REALM_ROLE(37, "deny-add-master-realm-role"),
        PROOF_DEADLINE(38, "proof-deadline"),
        CLEANUP(39, "cleanup"),
        TOKEN_CONTRACT(40, "token-contract"),
        REFRESH_CONTRACT(41, "refresh-contract");

        private final int exitCode;
        private final String label;

        AuthorityStage(int exitCode, String label) {
            this.exitCode = exitCode;
            this.label = label;
        }
    }

    static final class AuthorityProofRefusal extends Exception {
        private final AuthorityStage stage;

        AuthorityProofRefusal(AuthorityStage stage) {
            this.stage = stage;
        }

        AuthorityStage stage() {
            return stage;
        }
    }

    @FunctionalInterface
    interface AuthorityAction {
        void run() throws Exception;
    }

    @FunctionalInterface
    private interface AuthoritySupplier<T> {
        T get() throws Exception;
    }

    private SynvedaKeycloakProjection() {}

    public static void main(String[] args) {
        try {
            if (args.length == 1 && args[0].equals("bootstrap-refused")) {
                verifyBootstrapLoginRefused();
                return;
            }
            if (args.length == 3 && args[0].equals("admin-authority-login")) {
                verifyAdminAuthorityLogin(args[1], args[2]);
                return;
            }
            if (args.length == 2 && args[0].equals("admin-session-close")) {
                closeAdminSession(read(args[1]));
                return;
            }
            if (args.length == 2
                && args[0].equals("admin-session-settle-failed")) {
                settleFailedAdminSession(read(args[1]));
                return;
            }
            if (args.length < 2) {
                throw new IllegalArgumentException();
            }
            if (args[0].equals("origin")) {
                requireArgs(args, 2);
                verifyOrigin(args[1]);
                return;
            }
            JsonNode input = read(args[1]);
            switch (args[0]) {
                case "realm" -> {
                    requireArgs(args, 4);
                    verifyRealm(input, exactBoolean(args[2]), args[3]);
                }
                case "realm-state" -> {
                    requireArgs(args, 3);
                    verifyRealmState(input, exactBoolean(args[2]));
                }
                case "target-realm" -> {
                    requireArgs(args, 2);
                    verifyTargetRealm(input);
                }
                case "client" -> {
                    requireArgs(args, 3);
                    verifyClient(input, args[2]);
                }
                case "user-profile" -> {
                    requireArgs(args, 2);
                    verifyUserProfile(input);
                }
                case "client-id" -> {
                    requireArgs(args, 3);
                    namedId(input, "clientId", args[2], "client", true);
                }
                case "user-id" -> {
                    requireArgs(args, 3);
                    namedId(input, "username", args[2], "user", true);
                }
                case "demo-user-owned" -> {
                    requireArgs(args, 4);
                    verifyDemoUser(input, args[2], args[3], false);
                }
                case "demo-user" -> {
                    requireArgs(args, 4);
                    verifyDemoUser(input, args[2], args[3], true);
                }
                case "demo-user-state" -> {
                    requireArgs(args, 4);
                    demoUserState(input, args[2], args[3]);
                }
                case "demo-user-owned-id" -> {
                    requireArgs(args, 4);
                    verifyDemoUserOwnedId(input, args[2], args[3]);
                }
                case "demo-owned-users" -> {
                    requireArgs(args, 2);
                    verifyDemoOwnedUsers(input, null);
                }
                case "demo-owned-user-kind" -> {
                    requireArgs(args, 3);
                    verifyDemoOwnedUsers(input, args[2]);
                }
                case "demo-password-credential" -> {
                    requireArgs(args, 2);
                    verifyDemoPasswordCredential(input);
                }
                case "demo-group-members" -> {
                    requireArgs(args, 4);
                    verifyDemoGroupMembers(input, args[2], args[3]);
                }
                case "scope-id" -> {
                    requireArgs(args, 3);
                    namedId(input, "name", args[2], "scope", true);
                }
                case "scope-ids" -> {
                    requireArgs(args, 3);
                    scopeIds(input, args[2]);
                }
                case "mapper-ids" -> {
                    requireArgs(args, 2);
                    mapperIds(input);
                }
                case "mappers" -> {
                    requireArgs(args, 2);
                    verifyMappers(input);
                }
                case "group-id" -> {
                    requireArgs(args, 2);
                    groupId(input, false);
                }
                case "group" -> {
                    requireArgs(args, 2);
                    groupId(input, true);
                }
                case "roles" -> {
                    requireArgs(args, 2);
                    verifyRoles(input);
                }
                case "role-ids" -> {
                    requireArgs(args, 2);
                    roleIds(input);
                }
                case "object-ids" -> {
                    requireArgs(args, 2);
                    objectIds(input);
                }
                case "demo-object-ids" -> {
                    requireArgs(args, 2);
                    objectIds(input, MAX_DEMO_CLEANUP_ITEMS);
                }
                case "role-mapping-ids" -> {
                    requireArgs(args, 2);
                    roleMappingIds(input);
                }
                case "demo-role-mapping-ids" -> {
                    requireArgs(args, 2);
                    roleMappingIds(input, MAX_DEMO_CLEANUP_ITEMS);
                }
                case "group-ids" -> {
                    requireArgs(args, 2);
                    groupIds(input);
                }
                case "demo-group-ids" -> {
                    requireArgs(args, 2);
                    groupIds(input, MAX_DEMO_CLEANUP_ITEMS);
                }
                case "empty-array" -> {
                    requireArgs(args, 2);
                    emptyArray(input);
                }
                case "direct-role-mapping" -> {
                    requireArgs(args, 2);
                    verifyDirectRoleMapping(input);
                }
                case "empty-role-mapping" -> {
                    requireArgs(args, 2);
                    verifyEmptyRoleMapping(input);
                }
                case "effective-roles" -> {
                    requireArgs(args, 3);
                    verifyEffectiveRoles(input, args[2]);
                }
                case "effective-audit-role" -> {
                    requireArgs(args, 4);
                    verifyEffectiveAuditRole(input, args[2], args[3]);
                }
                case "realm-witness" -> {
                    requireArgs(args, 3);
                    verifyRealmWitness(input, args[2]);
                }
                case "realm-witness-patch" -> {
                    requireArgs(args, 5);
                    realmWitnessPatch(input, args[2], args[3], args[4]);
                }
                case "admin-token" -> {
                    requireArgs(args, 2);
                    verifyAdminToken(input);
                }
                case "admin-bootstrap-delete" -> {
                    requireArgs(args, 3);
                    deleteBootstrapUser(input, args[2]);
                }
                case "admin-quarantine" -> {
                    requireArgs(args, 2);
                    quarantineTargetRealm(input);
                }
                default -> throw new IllegalArgumentException();
            }
            flushOutput();
        } catch (AuthorityProofRefusal refused) {
            System.err.println(REFUSAL_MESSAGE);
            System.exit(refused.stage.exitCode);
        } catch (Throwable ignored) {
            System.err.println(REFUSAL_MESSAGE);
            System.exit(1);
        }
    }

    static void atAuthorityStage(
        AuthorityStage stage,
        AuthorityAction action
    ) throws AuthorityProofRefusal {
        try {
            action.run();
        } catch (AuthorityProofRefusal refused) {
            throw refused;
        } catch (Exception ignored) {
            throw new AuthorityProofRefusal(stage);
        }
    }

    private static <T> T atAuthorityStage(
        AuthorityStage stage,
        AuthoritySupplier<T> supplier
    ) throws AuthorityProofRefusal {
        try {
            return supplier.get();
        } catch (AuthorityProofRefusal refused) {
            throw refused;
        } catch (Exception ignored) {
            throw new AuthorityProofRefusal(stage);
        }
    }

    static Map<Integer, String> authorityStageContract() {
        Map<Integer, String> contract = new TreeMap<>();
        Set<String> labels = new HashSet<>();
        for (AuthorityStage stage : AuthorityStage.values()) {
            if (stage.exitCode < 20
                || stage.exitCode >= 64
                || !stage.label.matches("[a-z][a-z0-9-]{0,31}")
                || !labels.add(stage.label)
                || contract.put(stage.exitCode, stage.label) != null) {
                throw new IllegalStateException();
            }
        }
        if (contract.size() != AuthorityStage.values().length) {
            throw new IllegalStateException();
        }
        return Map.copyOf(contract);
    }

    static void runAuthorityProofWithCleanup(
        AuthorityAction proof,
        AuthorityAction cleanup
    ) throws Exception {
        try {
            proof.run();
        } finally {
            try {
                cleanup.run();
            } catch (Exception ignored) {
                throw new AuthorityProofRefusal(AuthorityStage.CLEANUP);
            }
        }
    }

    private static ObjectMapper strictJson() {
        JsonFactory factory = JsonFactory.builder()
            .enable(StreamReadFeature.STRICT_DUPLICATE_DETECTION)
            .build();
        ObjectMapper mapper = new ObjectMapper(factory);
        mapper.enable(DeserializationFeature.FAIL_ON_TRAILING_TOKENS);
        return mapper;
    }

    private static JsonNode read(String rawPath) throws Exception {
        Path path = Path.of(rawPath);
        long size = Files.size(path);
        if (size < 2 || size > MAX_INPUT_BYTES || !Files.isRegularFile(path)) {
            throw new IllegalArgumentException();
        }
        return JSON.readTree(Files.readAllBytes(path));
    }

    private static void verifyOrigin(String raw) throws Exception {
        URI uri = new URI(raw);
        String scheme = uri.getScheme();
        String path = uri.getRawPath();
        int port = uri.getPort();
        if (!("http".equalsIgnoreCase(scheme) || "https".equalsIgnoreCase(scheme))
            || uri.getHost() == null
            || uri.getRawUserInfo() != null
            || !(path == null || path.isEmpty() || path.equals("/"))
            || uri.getRawQuery() != null
            || uri.getRawFragment() != null
            || port == 0
            || port > 65535) {
            throw new IllegalArgumentException();
        }
    }

    private static void requireArgs(String[] args, int expected) {
        if (args.length != expected) {
            throw new IllegalArgumentException();
        }
    }

    private static boolean exactBoolean(String value) {
        return switch (value) {
            case "true" -> true;
            case "false" -> false;
            default -> throw new IllegalArgumentException();
        };
    }

    private static void emit(String value) {
        OUTPUT.add(value);
    }

    private static void flushOutput() {
        if (OUTPUT.isEmpty()) {
            return;
        }
        System.out.print(String.join("\n", OUTPUT) + "\n");
        if (System.out.checkError()) {
            throw new IllegalStateException();
        }
    }

    private static void verifyRealm(JsonNode node, boolean enabled, String sslRequired) {
        requireObject(node);
        text(node, "realm", "synveda");
        bool(node, "enabled", enabled);
        text(node, "sslRequired", sslRequired);
        bool(node, "registrationAllowed", false);
        bool(node, "registrationEmailAsUsername", false);
        bool(node, "rememberMe", false);
        bool(node, "verifyEmail", false);
        bool(node, "resetPasswordAllowed", false);
        bool(node, "editUsernameAllowed", false);
        bool(node, "loginWithEmailAllowed", true);
        bool(node, "duplicateEmailsAllowed", false);
        bool(node, "bruteForceProtected", true);
        bool(node, "permanentLockout", false);
        number(node, "maxFailureWaitSeconds", 900);
        number(node, "minimumQuickLoginWaitSeconds", 60);
        number(node, "waitIncrementSeconds", 60);
        number(node, "quickLoginCheckMilliSeconds", 1000);
        number(node, "maxDeltaTimeSeconds", 43200);
        number(node, "failureFactor", 5);
        number(node, "accessTokenLifespan", 300);
        number(node, "ssoSessionIdleTimeout", 1800);
        number(node, "ssoSessionMaxLifespan", 28800);
        bool(node, "revokeRefreshToken", true);
        number(node, "refreshTokenMaxReuse", 0);
        text(node, "defaultSignatureAlgorithm", "RS256");
        bool(node, "eventsEnabled", true);
        number(node, "eventsExpiration", 604800);
        bool(node, "adminEventsEnabled", true);
        bool(node, "adminEventsDetailsEnabled", false);
    }

    private static void verifyRealmState(JsonNode node, boolean enabled) {
        requireObject(node);
        text(node, "realm", "synveda");
        bool(node, "enabled", enabled);
    }

    private static void verifyClient(JsonNode node, String appUrl) {
        requireObject(node);
        text(node, "clientId", "synveda");
        bool(node, "enabled", true);
        text(node, "protocol", "openid-connect");
        bool(node, "publicClient", true);
        bool(node, "bearerOnly", false);
        bool(node, "standardFlowEnabled", true);
        bool(node, "implicitFlowEnabled", false);
        bool(node, "directAccessGrantsEnabled", false);
        bool(node, "serviceAccountsEnabled", false);
        boolOrMissingFalse(node, "authorizationServicesEnabled");
        bool(node, "consentRequired", false);
        bool(node, "fullScopeAllowed", false);
        bool(node, "frontchannelLogout", true);
        text(node, "rootUrl", appUrl);
        text(node, "baseUrl", appUrl + "/console/");
        stringSet(node, "redirectUris", Set.of(appUrl + "/auth/callback"));
        stringSet(node, "webOrigins", Set.of(appUrl));
        stringSet(node, "defaultClientScopes", Set.of("email", "profile"));
        stringSet(node, "optionalClientScopes", Set.of());
        JsonNode attributes = node.path("attributes");
        requireObject(attributes);
        if (attributes.size() != 10) {
            throw new IllegalArgumentException();
        }
        text(attributes, "pkce.code.challenge.method", "S256");
        text(attributes, "use.refresh.tokens", "true");
        text(attributes, "client_credentials.use_refresh_token", "false");
        text(attributes, "dpop.bound.access.tokens", "false");
        text(attributes, "exclude.session.state.from.auth.response", "false");
        text(attributes, "exclude.issuer.from.auth.response", "false");
        text(attributes, "realm_client", "false");
        text(attributes, "backchannel.logout.session.required", "true");
        text(attributes, "backchannel.logout.revoke.offline.tokens", "false");
        String secretCreation = requiredText(attributes.path("client.secret.creation.time"));
        if (!secretCreation.chars().allMatch(Character::isDigit)) {
            throw new IllegalArgumentException();
        }
    }

    static void verifyUserProfile(JsonNode node) {
        requireObject(node);
        Set<String> rootFields = new HashSet<>();
        node.fieldNames().forEachRemaining(rootFields::add);
        if (!rootFields.equals(Set.of("attributes", "groups"))
            && !rootFields.equals(Set.of(
                "attributes", "groups", "unmanagedAttributePolicy"
            ))) {
            throw new IllegalArgumentException();
        }
        JsonNode unmanagedPolicy = node.path("unmanagedAttributePolicy");
        if (!unmanagedPolicy.isMissingNode() && !unmanagedPolicy.isNull()) {
            throw new IllegalArgumentException();
        }
        JsonNode attributes = node.path("attributes");
        requireArray(attributes);
        if (attributes.size() != 6) {
            throw new IllegalArgumentException();
        }
        Map<String, JsonNode> byName = new HashMap<>();
        for (JsonNode attribute : attributes) {
            requireObject(attribute);
            String name = requiredText(attribute.path("name"));
            if (byName.put(name, attribute) != null) {
                throw new IllegalArgumentException();
            }
        }
        if (!byName.keySet().equals(Set.of(
            "username",
            "email",
            "firstName",
            "lastName",
            "synvedaDemoContract",
            "synvedaDemoKind"
        ))) {
            throw new IllegalArgumentException();
        }
        verifyBuiltInProfileAttribute(byName.get("username"), "username");
        verifyBuiltInProfileAttribute(byName.get("email"), "email");
        verifyBuiltInProfileAttribute(byName.get("firstName"), "firstName");
        verifyBuiltInProfileAttribute(byName.get("lastName"), "lastName");
        verifyMarkerProfileAttribute(
            byName.get("synvedaDemoContract"),
            "synvedaDemoContract",
            "Synveda demo contract",
            13,
            13,
            Set.of("cpr45-demo-v1")
        );
        verifyMarkerProfileAttribute(
            byName.get("synvedaDemoKind"),
            "synvedaDemoKind",
            "Synveda demo kind",
            5,
            6,
            Set.of("admin", "member")
        );

        JsonNode groups = node.path("groups");
        requireArray(groups);
        if (groups.size() != 1) {
            throw new IllegalArgumentException();
        }
        JsonNode group = groups.get(0);
        requireObject(group);
        exactFields(group, Set.of("name", "displayHeader", "displayDescription"));
        text(group, "name", "user-metadata");
        text(group, "displayHeader", "User metadata");
        text(group, "displayDescription", "Attributes, which refer to user metadata");
    }

    private static void verifyBuiltInProfileAttribute(JsonNode node, String name) {
        Set<String> requiredFields = name.equals("username")
            ? Set.of("name", "displayName", "permissions", "validations")
            : Set.of(
                "name",
                "displayName",
                "required",
                "permissions",
                "validations"
            );
        Set<String> actualFields = new HashSet<>();
        node.fieldNames().forEachRemaining(actualFields::add);
        if (!actualFields.equals(requiredFields)) {
            Set<String> withMultivalued = new HashSet<>(requiredFields);
            withMultivalued.add("multivalued");
            if (!actualFields.equals(withMultivalued)) {
                throw new IllegalArgumentException();
            }
        }
        text(node, "name", name);
        text(node, "displayName", "${" + name + "}");
        boolOrMissingFalse(node, "multivalued");
        verifyProfilePermissions(node.path("permissions"), Set.of("admin", "user"));
        if (!name.equals("username")) {
            JsonNode required = node.path("required");
            requireObject(required);
            exactFields(required, Set.of("roles"));
            stringSet(required, "roles", Set.of("user"));
        }
        JsonNode validations = node.path("validations");
        requireObject(validations);
        switch (name) {
            case "username" -> {
                exactFields(validations, Set.of(
                    "length",
                    "username-prohibited-characters",
                    "up-username-not-idn-homograph"
                ));
                verifyLengthValidation(validations.path("length"), 3, 255);
                requireEmptyObject(validations.path("username-prohibited-characters"));
                requireEmptyObject(validations.path("up-username-not-idn-homograph"));
            }
            case "email" -> {
                exactFields(validations, Set.of("email", "length"));
                requireEmptyObject(validations.path("email"));
                verifyLengthValidation(validations.path("length"), null, 255);
            }
            case "firstName", "lastName" -> {
                exactFields(validations, Set.of("length", "person-name-prohibited-characters"));
                verifyLengthValidation(validations.path("length"), null, 255);
                requireEmptyObject(validations.path("person-name-prohibited-characters"));
            }
            default -> throw new IllegalArgumentException();
        }
    }

    private static void verifyMarkerProfileAttribute(
        JsonNode node,
        String name,
        String displayName,
        int minLength,
        int maxLength,
        Set<String> options
    ) {
        exactFields(node, Set.of(
            "name", "displayName", "multivalued", "permissions", "validations"
        ));
        text(node, "name", name);
        text(node, "displayName", displayName);
        bool(node, "multivalued", false);
        verifyProfilePermissions(node.path("permissions"), Set.of("admin"));
        JsonNode validations = node.path("validations");
        requireObject(validations);
        exactFields(validations, Set.of("length", "options"));
        verifyLengthValidation(validations.path("length"), minLength, maxLength);
        JsonNode optionValidation = validations.path("options");
        requireObject(optionValidation);
        exactFields(optionValidation, Set.of("options"));
        stringSet(optionValidation, "options", options);
    }

    private static void verifyProfilePermissions(JsonNode node, Set<String> roles) {
        requireObject(node);
        exactFields(node, Set.of("view", "edit"));
        stringSet(node, "view", roles);
        stringSet(node, "edit", roles);
    }

    private static void verifyLengthValidation(JsonNode node, Integer min, int max) {
        requireObject(node);
        exactFields(node, min == null ? Set.of("max") : Set.of("min", "max"));
        if (min != null) {
            number(node, "min", min);
        }
        number(node, "max", max);
    }

    private static void requireEmptyObject(JsonNode node) {
        requireObject(node);
        if (!node.isEmpty()) {
            throw new IllegalArgumentException();
        }
    }

    static void verifyUserProfile(byte[] input) {
        try {
            verifyUserProfile(JSON.readTree(input));
        } catch (IllegalArgumentException refused) {
            throw refused;
        } catch (Exception ignored) {
            throw new IllegalArgumentException();
        }
    }

    private static void namedId(
        JsonNode node,
        String field,
        String expected,
        String label,
        boolean allowMissing
    ) {
        requireArray(node);
        List<JsonNode> matches = new ArrayList<>();
        for (JsonNode candidate : node) {
            requireObject(candidate);
            String actual = requiredText(candidate.path(field));
            if (actual.equalsIgnoreCase(expected)) {
                matches.add(candidate);
            }
        }
        if (matches.isEmpty() && allowMissing) {
            emit(label + "=missing");
            return;
        }
        if (matches.size() != 1 || !requiredText(matches.get(0).path(field)).equals(expected)) {
            throw new IllegalArgumentException();
        }
        emit(label + "=" + uuid(matches.get(0).path("id")));
    }

    static void verifyDemoUser(
        JsonNode node,
        String expectedUsername,
        String expectedKind,
        boolean complete
    ) {
        requireObject(node);
        uuid(node.path("id"));
        text(node, "username", expectedUsername);
        verifyDemoOwnership(node, expectedKind);
        if (!complete) {
            return;
        }
        bool(node, "enabled", true);
        bool(node, "emailVerified", true);
        text(node, "email", expectedKind + "@demo.synveda.invalid");
        text(node, "firstName", "Synveda");
        text(node, "lastName", expectedKind.equals("admin") ? "Demo Admin" : "Demo Member");
        emptyArray(node.path("requiredActions"));
    }

    private static void verifyDemoOwnership(JsonNode node, String expectedKind) {
        if (!Set.of("admin", "member").contains(expectedKind)) {
            throw new IllegalArgumentException();
        }
        JsonNode attributes = node.path("attributes");
        requireObject(attributes);
        if (attributes.size() != 2) {
            throw new IllegalArgumentException();
        }
        stringSet(attributes, "synvedaDemoContract", Set.of("cpr45-demo-v1"));
        stringSet(attributes, "synvedaDemoKind", Set.of(expectedKind));
    }

    private static void verifyDemoUserOwnedId(
        JsonNode node,
        String expectedKind,
        String expectedId
    ) {
        requireObject(node);
        if (!uuid(node.path("id")).equals(exactUuid(expectedId))) {
            throw new IllegalArgumentException();
        }
        requiredText(node.path("username"));
        verifyDemoOwnership(node, expectedKind);
    }

    private static void verifyDemoOwnedUsers(JsonNode node, String expectedKind) {
        requireArray(node);
        if (node.size() > 2 || (expectedKind != null && node.size() > 1)) {
            throw new IllegalArgumentException();
        }
        Map<String, String> ids = new TreeMap<>();
        for (JsonNode user : node) {
            requireObject(user);
            requiredText(user.path("username"));
            JsonNode kindNode = user.path("attributes").path("synvedaDemoKind");
            requireArray(kindNode);
            if (kindNode.size() != 1) {
                throw new IllegalArgumentException();
            }
            String kind = requiredText(kindNode.get(0));
            if (expectedKind != null && !kind.equals(expectedKind)) {
                throw new IllegalArgumentException();
            }
            verifyDemoOwnership(user, kind);
            if (ids.put(kind, uuid(user.path("id"))) != null) {
                throw new IllegalArgumentException();
            }
        }
        for (String kind : List.of("admin", "member")) {
            if (ids.containsKey(kind)) {
                emit(kind + "=" + ids.get(kind));
            }
        }
    }

    static void verifyDemoUser(
        byte[] input,
        String expectedUsername,
        String expectedKind,
        boolean complete
    ) {
        try {
            verifyDemoUser(JSON.readTree(input), expectedUsername, expectedKind, complete);
        } catch (IllegalArgumentException refused) {
            throw refused;
        } catch (Exception ignored) {
            throw new IllegalArgumentException();
        }
    }

    static void demoUserState(JsonNode node, String expectedUsername, String expectedKind) {
        requireObject(node);
        uuid(node.path("id"));
        text(node, "username", expectedUsername);
        JsonNode attributes = node.path("attributes");
        if (attributes.isMissingNode() || attributes.isNull()) {
            emit("demo=foreign");
            return;
        }
        requireObject(attributes);
        int managedAttributes = 0;
        var names = attributes.fieldNames();
        while (names.hasNext()) {
            if (names.next().toLowerCase(java.util.Locale.ROOT).startsWith("synvedademo")) {
                managedAttributes += 1;
            }
        }
        if (managedAttributes == 0) {
            emit("demo=foreign");
            return;
        }
        verifyDemoUser(node, expectedUsername, expectedKind, false);
        emit("demo=owned");
    }

    static void demoUserState(byte[] input, String expectedUsername, String expectedKind) {
        try {
            demoUserState(JSON.readTree(input), expectedUsername, expectedKind);
        } catch (IllegalArgumentException refused) {
            throw refused;
        } catch (Exception ignored) {
            throw new IllegalArgumentException();
        }
    }

    static void verifyDemoGroupMembers(
        JsonNode node,
        String expectedUserId,
        String expectedUsername
    ) {
        requireArray(node);
        if (node.size() != 1) {
            throw new IllegalArgumentException();
        }
        JsonNode member = node.get(0);
        requireObject(member);
        if (!uuid(member.path("id")).equals(exactUuid(expectedUserId))) {
            throw new IllegalArgumentException();
        }
        text(member, "username", expectedUsername);
        if (member.has("enabled")) {
            bool(member, "enabled", true);
        }
    }

    static void verifyDemoGroupMembers(
        byte[] input,
        String expectedUserId,
        String expectedUsername
    ) {
        try {
            verifyDemoGroupMembers(JSON.readTree(input), expectedUserId, expectedUsername);
        } catch (IllegalArgumentException refused) {
            throw refused;
        } catch (Exception ignored) {
            throw new IllegalArgumentException();
        }
    }

    static void verifyDemoPasswordCredential(JsonNode node) {
        requireArray(node);
        if (node.size() != 1) {
            throw new IllegalArgumentException();
        }
        JsonNode credential = node.get(0);
        requireObject(credential);
        uuid(credential.path("id"));
        text(credential, "type", "password");
    }

    static void verifyDemoPasswordCredential(byte[] input) {
        try {
            verifyDemoPasswordCredential(JSON.readTree(input));
        } catch (IllegalArgumentException refused) {
            throw refused;
        } catch (Exception ignored) {
            throw new IllegalArgumentException();
        }
    }

    private static void mapperIds(JsonNode node) {
        requireArray(node);
        Map<String, String> desired = new HashMap<>();
        List<String> extras = new ArrayList<>();
        for (JsonNode mapper : node) {
            requireObject(mapper);
            String id = uuid(mapper.path("id"));
            String name = requiredText(mapper.path("name"));
            if (name.equals("synveda-audience") || name.equals("synveda-groups")) {
                if (desired.put(name, id) != null) {
                    throw new IllegalArgumentException();
                }
            } else {
                extras.add(id);
            }
        }
        extras.sort(String::compareTo);
        emit("audience=" + desired.getOrDefault("synveda-audience", "missing"));
        emit("groups=" + desired.getOrDefault("synveda-groups", "missing"));
        for (String extra : extras) {
            emit("extra=" + extra);
        }
    }

    private static void scopeIds(JsonNode node, String kind) {
        requireArray(node);
        List<String> desiredOrder = switch (kind) {
            case "default" -> List.of("email", "profile");
            case "optional" -> List.of();
            default -> throw new IllegalArgumentException();
        };
        Set<String> desired = Set.copyOf(desiredOrder);
        Map<String, String> present = new HashMap<>();
        Set<String> foldedNames = new HashSet<>();
        Set<String> ids = new HashSet<>();
        List<String> extras = new ArrayList<>();
        for (JsonNode scope : node) {
            requireObject(scope);
            String id = uuid(scope.path("id"));
            if (!ids.add(id)) {
                throw new IllegalArgumentException();
            }
            String name = requiredText(scope.path("name"));
            if (!foldedNames.add(name.toLowerCase(java.util.Locale.ROOT))) {
                throw new IllegalArgumentException();
            }
            if (desired.contains(name)) {
                present.put(name, id);
            } else {
                extras.add(id);
            }
        }
        extras.sort(String::compareTo);
        for (String extra : extras) {
            emit("extra=" + extra);
        }
        for (String name : desiredOrder) {
            emit(name + "=" + present.getOrDefault(name, "missing"));
        }
    }

    private static void verifyMappers(JsonNode node) {
        requireArray(node);
        if (node.size() != 2) {
            throw new IllegalArgumentException();
        }
        Set<String> names = new HashSet<>();
        for (JsonNode mapper : node) {
            requireObject(mapper);
            uuid(mapper.path("id"));
            String name = requiredText(mapper.path("name"));
            if (!names.add(name)) {
                throw new IllegalArgumentException();
            }
            text(mapper, "protocol", "openid-connect");
            bool(mapper, "consentRequired", false);
            JsonNode config = mapper.path("config");
            requireObject(config);
            if (name.equals("synveda-groups")) {
                text(mapper, "protocolMapper", "oidc-group-membership-mapper");
                exactStringMap(config, Map.of(
                    "claim.name", "groups",
                    "full.path", "false",
                    "id.token.claim", "true",
                    "access.token.claim", "true",
                    "userinfo.token.claim", "false",
                    "introspection.token.claim", "false"
                ));
            } else if (name.equals("synveda-audience")) {
                text(mapper, "protocolMapper", "oidc-audience-mapper");
                exactStringMap(config, Map.of(
                    "included.custom.audience", "synveda-api",
                    "id.token.claim", "false",
                    "access.token.claim", "true",
                    "userinfo.token.claim", "false",
                    "introspection.token.claim", "false"
                ));
            } else {
                throw new IllegalArgumentException();
            }
        }
        if (!names.equals(Set.of("synveda-audience", "synveda-groups"))) {
            throw new IllegalArgumentException();
        }
    }

    private static void groupId(JsonNode node, boolean verifyAttributes) {
        requireArray(node);
        List<JsonNode> matches = new ArrayList<>();
        collectAdminGroups(node, matches);
        if (matches.isEmpty()) {
            if (verifyAttributes) {
                throw new IllegalArgumentException();
            }
            emit("group=missing");
            return;
        }
        if (matches.size() != 1) {
            throw new IllegalArgumentException();
        }
        JsonNode group = matches.get(0);
        text(group, "name", "synveda-admins");
        text(group, "path", "/synveda-admins");
        String id = uuid(group.path("id"));
        if (verifyAttributes) {
            JsonNode attributes = group.path("attributes");
            requireObject(attributes);
            if (!attributes.isEmpty()) {
                throw new IllegalArgumentException();
            }
            JsonNode subGroups = group.path("subGroups");
            if (!subGroups.isMissingNode() && !subGroups.isNull()) {
                emptyArray(subGroups);
            }
            JsonNode subGroupCount = group.path("subGroupCount");
            if (!subGroupCount.isMissingNode() && !subGroupCount.isNull()) {
                number(group, "subGroupCount", 0);
            }
            JsonNode realmRoles = group.path("realmRoles");
            if (!realmRoles.isMissingNode() && !realmRoles.isNull()) {
                emptyArray(realmRoles);
            }
            JsonNode clientRoles = group.path("clientRoles");
            if (!clientRoles.isMissingNode() && !clientRoles.isNull()) {
                requireObject(clientRoles);
                if (!clientRoles.isEmpty()) {
                    throw new IllegalArgumentException();
                }
            }
        }
        emit("group=" + id);
    }

    private static void collectAdminGroups(JsonNode groups, List<JsonNode> matches) {
        requireArray(groups);
        for (JsonNode group : groups) {
            requireObject(group);
            String name = requiredText(group.path("name"));
            if (name.equalsIgnoreCase("synveda-admins")) {
                matches.add(group);
            }
            JsonNode children = group.path("subGroups");
            if (!children.isMissingNode() && !children.isNull()) {
                collectAdminGroups(children, matches);
            }
        }
    }

    private static void verifyRoles(JsonNode node) {
        requireArray(node);
        Set<String> roles = new HashSet<>();
        for (JsonNode role : node) {
            requireObject(role);
            if (!roles.add(requiredText(role.path("name")))) {
                throw new IllegalArgumentException();
            }
        }
        if (!roles.equals(CONVERGENCE_ROLES)) {
            throw new IllegalArgumentException();
        }
    }

    private static void roleIds(JsonNode node) {
        requireArray(node);
        Set<String> ids = new HashSet<>();
        for (JsonNode role : node) {
            requireObject(role);
            String id = uuid(role.path("id"));
            if (!ids.add(id)) {
                throw new IllegalArgumentException();
            }
        }
        ids.stream().sorted().forEach(id -> emit("role=" + id));
    }

    private static void objectIds(JsonNode node) {
        objectIds(node, Integer.MAX_VALUE);
    }

    private static void objectIds(JsonNode node, int maximumItems) {
        requireArray(node);
        if (node.size() > maximumItems) {
            throw new IllegalArgumentException();
        }
        Set<String> ids = new HashSet<>();
        for (JsonNode object : node) {
            requireObject(object);
            if (!ids.add(uuid(object.path("id")))) {
                throw new IllegalArgumentException();
            }
        }
        ids.stream().sorted().forEach(id -> emit("id=" + id));
    }

    private static void roleMappingIds(JsonNode node) {
        roleMappingIds(node, Integer.MAX_VALUE);
    }

    private static void roleMappingIds(JsonNode node, int maximumItems) {
        requireObject(node);
        Set<String> roleIds = new HashSet<>();
        List<String> projection = new ArrayList<>();
        JsonNode realmMappings = node.path("realmMappings");
        if (!realmMappings.isMissingNode() && !realmMappings.isNull()) {
            requireArray(realmMappings);
            for (JsonNode role : realmMappings) {
                requireObject(role);
                String id = uuid(role.path("id"));
                if (!roleIds.add(id)) {
                    throw new IllegalArgumentException();
                }
                projection.add("realm=" + id);
            }
        }

        JsonNode clientMappings = node.path("clientMappings");
        if (!clientMappings.isMissingNode() && !clientMappings.isNull()) {
            requireObject(clientMappings);
            Set<String> clientIds = new HashSet<>();
            for (JsonNode mapping : clientMappings) {
                requireObject(mapping);
                String clientId = uuid(mapping.path("id"));
                if (!clientIds.add(clientId)) {
                    throw new IllegalArgumentException();
                }
                JsonNode mappings = mapping.path("mappings");
                requireArray(mappings);
                for (JsonNode role : mappings) {
                    requireObject(role);
                    String roleId = uuid(role.path("id"));
                    if (!roleIds.add(roleId)) {
                        throw new IllegalArgumentException();
                    }
                    projection.add("client=" + clientId + ":" + roleId);
                }
            }
        }
        if (projection.size() > maximumItems) {
            throw new IllegalArgumentException();
        }
        projection.sort(String::compareTo);
        projection.forEach(SynvedaKeycloakProjection::emit);
    }

    private static void groupIds(JsonNode node) {
        groupIds(node, Integer.MAX_VALUE);
    }

    private static void groupIds(JsonNode node, int maximumItems) {
        requireArray(node);
        if (node.size() > maximumItems) {
            throw new IllegalArgumentException();
        }
        Set<String> ids = new HashSet<>();
        for (JsonNode group : node) {
            requireObject(group);
            String id = uuid(group.path("id"));
            if (!ids.add(id)) {
                throw new IllegalArgumentException();
            }
        }
        ids.stream().sorted().forEach(id -> emit("group=" + id));
    }

    private static void emptyArray(JsonNode node) {
        requireArray(node);
        if (!node.isEmpty()) {
            throw new IllegalArgumentException();
        }
    }

    private static void verifyDirectRoleMapping(JsonNode node) {
        requireObject(node);
        JsonNode realmMappings = node.path("realmMappings");
        if (!realmMappings.isMissingNode() && !realmMappings.isNull()) {
            emptyArray(realmMappings);
        }
        JsonNode clientMappings = node.path("clientMappings");
        requireObject(clientMappings);
        if (clientMappings.size() != 2) {
            throw new IllegalArgumentException();
        }
        String targetClientId = null;
        String auditClientId = null;
        String auditRoleId = null;
        Set<String> directIds = new HashSet<>();
        for (Map.Entry<String, JsonNode> entry : clientMappings.properties()) {
            JsonNode mapping = entry.getValue();
            requireObject(mapping);
            String clientName = requiredText(mapping.path("client"));
            if (!entry.getKey().equals(clientName)) {
                throw new IllegalArgumentException();
            }
            String clientId = uuid(mapping.path("id"));
            if (!directIds.add(clientId)) {
                throw new IllegalArgumentException();
            }
            if (clientName.equals("synveda-realm")) {
                if (targetClientId != null) {
                    throw new IllegalArgumentException();
                }
                for (String roleId : verifyConvergenceRoles(
                    mapping.path("mappings"), clientId
                )) {
                    if (!directIds.add(roleId)) {
                        throw new IllegalArgumentException();
                    }
                }
                targetClientId = clientId;
            } else if (clientName.equals("master-realm")) {
                if (auditClientId != null) {
                    throw new IllegalArgumentException();
                }
                auditRoleId = verifyDirectAuditRole(
                    mapping.path("mappings"), clientId
                );
                if (!directIds.add(auditRoleId)) {
                    throw new IllegalArgumentException();
                }
                auditClientId = clientId;
            } else {
                throw new IllegalArgumentException();
            }
        }
        if (targetClientId == null || auditClientId == null || auditRoleId == null) {
            throw new IllegalArgumentException();
        }
        emit("target-client=" + targetClientId);
        emit("audit-client=" + auditClientId);
        emit("audit-role=" + auditRoleId);
    }

    static void verifyEmptyRoleMapping(JsonNode node) {
        requireObject(node);
        JsonNode realmMappings = node.path("realmMappings");
        if (!realmMappings.isMissingNode() && !realmMappings.isNull()) {
            emptyArray(realmMappings);
        }
        JsonNode clientMappings = node.path("clientMappings");
        if (!clientMappings.isMissingNode() && !clientMappings.isNull()) {
            requireObject(clientMappings);
            if (!clientMappings.isEmpty()) {
                throw new IllegalArgumentException();
            }
        }
    }

    static void verifyEmptyRoleMapping(byte[] input) {
        try {
            verifyEmptyRoleMapping(JSON.readTree(input));
        } catch (IllegalArgumentException refused) {
            throw refused;
        } catch (Exception ignored) {
            throw new IllegalArgumentException();
        }
    }

    private static void verifyEffectiveRoles(JsonNode node, String rawClientId) {
        verifyConvergenceRoles(node, exactUuid(rawClientId));
    }

    private static void verifyEffectiveAuditRole(
        JsonNode node,
        String rawClientId,
        String rawAuditRoleId
    ) {
        verifyEffectiveAuditRoles(
            node,
            exactUuid(rawClientId),
            exactUuid(rawAuditRoleId)
        );
    }

    private static String verifyDirectAuditRole(JsonNode node, String clientId) {
        requireArray(node);
        if (node.size() != 1) {
            throw new IllegalArgumentException();
        }
        JsonNode role = node.get(0);
        requireObject(role);
        String roleId = uuid(role.path("id"));
        text(role, "name", "view-users");
        bool(role, "clientRole", true);
        bool(role, "composite", true);
        text(role, "containerId", clientId);
        return roleId;
    }

    private static void verifyEffectiveAuditRoles(
        JsonNode node,
        String clientId,
        String directAuditRoleId
    ) {
        requireArray(node);
        if (node.size() != EFFECTIVE_AUDIT_ROLES.size()
            || clientId.equals(directAuditRoleId)) {
            throw new IllegalArgumentException();
        }
        Set<String> names = new HashSet<>();
        Set<String> ids = new HashSet<>();
        for (JsonNode role : node) {
            requireObject(role);
            String roleId = uuid(role.path("id"));
            if (roleId.equals(clientId) || !ids.add(roleId)) {
                throw new IllegalArgumentException();
            }
            String name = requiredText(role.path("name"));
            if (!names.add(name)) {
                throw new IllegalArgumentException();
            }
            bool(role, "clientRole", true);
            bool(role, "composite", name.equals("view-users"));
            text(role, "containerId", clientId);
            if (name.equals("view-users") && !roleId.equals(directAuditRoleId)) {
                throw new IllegalArgumentException();
            }
        }
        if (!names.equals(EFFECTIVE_AUDIT_ROLES)) {
            throw new IllegalArgumentException();
        }
    }

    private static Set<String> verifyConvergenceRoles(JsonNode node, String clientId) {
        requireArray(node);
        if (node.size() != CONVERGENCE_ROLES.size()) {
            throw new IllegalArgumentException();
        }
        Set<String> names = new HashSet<>();
        Set<String> ids = new HashSet<>();
        for (JsonNode role : node) {
            requireObject(role);
            if (!ids.add(uuid(role.path("id")))) {
                throw new IllegalArgumentException();
            }
            String name = requiredText(role.path("name"));
            if (!names.add(name)) {
                throw new IllegalArgumentException();
            }
            bool(role, "clientRole", true);
            bool(role, "composite", false);
            text(role, "containerId", clientId);
        }
        if (!names.equals(CONVERGENCE_ROLES)) {
            throw new IllegalArgumentException();
        }
        return ids;
    }

    private static void verifyTargetRealm(JsonNode node) {
        requireObject(node);
        text(node, "realm", "synveda");
    }

    private static void verifyRealmWitness(JsonNode node, String expectedState) {
        List<String> witness = managedWitness(node);
        if (expectedState.equals("absent")) {
            if (!witness.isEmpty()) {
                throw new IllegalArgumentException();
            }
            return;
        }
        if (witness.size() != 3 || !witness.get(0).equals(expectedState)) {
            throw new IllegalArgumentException();
        }
        emit("bootstrap=" + witness.get(1));
        emit("permanent=" + witness.get(2));
    }

    private static List<String> managedWitness(JsonNode node) {
        requireObject(node);
        text(node, "realm", "synveda");
        JsonNode attributes = node.path("attributes");
        int managedCount = 0;
        if (!attributes.isMissingNode() && !attributes.isNull()) {
            requireObject(attributes);
            for (Map.Entry<String, JsonNode> attribute : attributes.properties()) {
                if (attribute.getKey().toLowerCase(java.util.Locale.ROOT).startsWith("synveda")) {
                    if (!MANAGED_ATTRIBUTE_KEYS.contains(attribute.getKey())) {
                        throw new IllegalArgumentException();
                    }
                    managedCount += 1;
                }
            }
        }
        if (managedCount == 0) {
            return List.of();
        }
        if (managedCount != MANAGED_ATTRIBUTE_KEYS.size()) {
            throw new IllegalArgumentException();
        }
        text(attributes, MANAGED_CONTRACT_KEY, "cpr45-keycloak-realm-v3");
        String state = requiredText(attributes.path(RETIREMENT_STATE_KEY));
        if (!(state.equals("pending") || state.equals("complete"))) {
            throw new IllegalArgumentException();
        }
        String bootstrapId = uuid(attributes.path(BOOTSTRAP_USER_ID_KEY));
        String convergenceId = uuid(attributes.path(CONVERGENCE_USER_ID_KEY));
        if (bootstrapId.equals(convergenceId)) {
            throw new IllegalArgumentException();
        }
        return List.of(state, bootstrapId, convergenceId);
    }

    private static void realmWitnessPatch(
        JsonNode node,
        String state,
        String rawBootstrapId,
        String rawConvergenceId
    ) throws Exception {
        managedWitness(node);
        if (!(state.equals("pending") || state.equals("complete"))) {
            throw new IllegalArgumentException();
        }
        String bootstrapId = exactUuid(rawBootstrapId);
        String convergenceId = exactUuid(rawConvergenceId);
        if (bootstrapId.equals(convergenceId)) {
            throw new IllegalArgumentException();
        }
        TreeMap<String, String> attributes = new TreeMap<>();
        JsonNode current = node.path("attributes");
        if (!current.isMissingNode() && !current.isNull()) {
            for (Map.Entry<String, JsonNode> attribute : current.properties()) {
                if (!attribute.getValue().isTextual()) {
                    throw new IllegalArgumentException();
                }
                attributes.put(attribute.getKey(), attribute.getValue().textValue());
            }
        }
        attributes.put(MANAGED_CONTRACT_KEY, "cpr45-keycloak-realm-v3");
        attributes.put(RETIREMENT_STATE_KEY, state);
        attributes.put(BOOTSTRAP_USER_ID_KEY, bootstrapId);
        attributes.put(CONVERGENCE_USER_ID_KEY, convergenceId);
        emit(JSON.writeValueAsString(Map.of("attributes", attributes)));
    }

    private static void verifyAdminToken(JsonNode config) throws Exception {
        requireObject(config);
        JsonNode claims = decodeAccessToken(config);
        if (claims.size() != 8) {
            throw new IllegalArgumentException();
        }
        text(claims, "azp", "admin-cli");
        text(claims, "typ", "Bearer");
        requiredText(claims.path("jti"));
        requiredText(claims.path("iss"));
        requiredText(claims.path("sid"));
        stringWords(claims, "scope", Set.of("email", "profile"));
        JsonNode issuedAt = claims.path("iat");
        JsonNode expiresAt = claims.path("exp");
        if (!issuedAt.isIntegralNumber()
            || !expiresAt.isIntegralNumber()
            || !issuedAt.canConvertToLong()
            || !expiresAt.canConvertToLong()
            || issuedAt.longValue() <= 0
            || expiresAt.longValue() <= issuedAt.longValue()) {
            throw new IllegalArgumentException();
        }
        if (claims.has("realm_access") || claims.has("resource_access")) {
            throw new IllegalArgumentException();
        }
    }

    static void verifyAdminSessionConfig(byte[] body) throws Exception {
        if (body.length < 2 || body.length > MAX_INPUT_BYTES) {
            throw new IllegalArgumentException();
        }
        JsonNode config;
        try {
            config = JSON.readTree(body);
        } catch (Exception ignored) {
            throw new IllegalArgumentException();
        }
        adminSessionRefreshToken(config);
    }

    private static String adminSessionRefreshToken(JsonNode config)
        throws Exception {
        requireObject(config);
        if (config.size() != 3) {
            throw new IllegalArgumentException();
        }
        text(config, "serverUrl", ADMIN_SERVER_URL);
        text(config, "realm", "master");
        JsonNode endpoints = config.path("endpoints");
        requireObject(endpoints);
        if (endpoints.size() != 1) {
            throw new IllegalArgumentException();
        }
        JsonNode realms = endpoints.path(ADMIN_SERVER_URL);
        requireObject(realms);
        if (realms.size() != 1) {
            throw new IllegalArgumentException();
        }
        JsonNode session = realms.path("master");
        requireObject(session);
        if (session.size() != 6) {
            throw new IllegalArgumentException();
        }
        text(session, "clientId", ADMIN_CLIENT);
        text(session, "grantTypeForAuthentication", "password");
        String accessToken = compactToken(session.path("token"));
        String refreshToken = compactToken(session.path("refreshToken"));
        long expiresAt = integralLong(session.path("expiresAt"));
        long refreshExpiresAt = integralLong(session.path("refreshExpiresAt"));
        if (expiresAt <= 0
            || accessToken.equals(refreshToken)
            || refreshExpiresAt <= expiresAt) {
            throw new IllegalArgumentException();
        }
        verifyAdminToken(config);
        decodeJwt(refreshToken);
        return refreshToken;
    }

    private static void closeAdminSession(JsonNode config) throws Exception {
        String refreshToken = adminSessionRefreshToken(config);
        revokeAndVerifyRefreshRefused(adminHttpClient(), refreshToken);
    }

    private static void settleFailedAdminSession(JsonNode config)
        throws Exception {
        closeAdminSession(config);
    }

    private static void verifyAdminAuthorityLogin(
        String rawUserId,
        String rawBootstrapUserId
    ) throws Exception {
        String userId = exactUuid(rawUserId);
        String username = requiredEnvironment("SYNVEDA_PROBE_USERNAME");
        String bootstrapUsername = requiredEnvironment(
            "SYNVEDA_PROBE_BOOTSTRAP_USERNAME"
        );
        String expectedBootstrapUserId = rawBootstrapUserId.equals("retired")
            ? "retired"
            : exactUuid(rawBootstrapUserId);
        String password = requiredEnvironment("KC_CLI_PASSWORD");
        String expectedIssuer = requiredEnvironment("SYNVEDA_PROBE_ISSUER");
        verifyAdminIssuer(expectedIssuer);
        long proofDeadlineNanos = Math.addExact(
            System.nanoTime(),
            AUTHORITY_PROOF_BUDGET.toNanos()
        );
        String form = "grant_type=password&client_id=" + ADMIN_CLIENT
            + "&scope=openid&username="
            + URLEncoder.encode(username, StandardCharsets.UTF_8)
            + "&password="
            + URLEncoder.encode(password, StandardCharsets.UTF_8);
        HttpClient client = adminHttpClient();
        long tokenRequestStartedAt = Instant.now().getEpochSecond();
        BoundedResponse tokenResponse = atAuthorityStage(
            AuthorityStage.TOKEN_HTTP,
            () -> sendBounded(
                client,
                formPost(ADMIN_REALM_URL + "/protocol/openid-connect/token", form),
                MAX_TOKEN_RESPONSE_BYTES,
                proofDeadlineNanos
            )
        );
        long tokenResponseReceivedAt = Instant.now().getEpochSecond();
        AuthorityTokenGrant tokenGrant = atAuthorityStage(
            AuthorityStage.TOKEN_ENVELOPE,
            () -> parseAuthorityTokenGrant(
                tokenResponse.statusCode(),
                tokenResponse.body()
            )
        );
        String refreshToken = tokenGrant.refreshToken();
        runAuthorityProofWithCleanup(() -> {
            AuthorityTokens tokens = atAuthorityStage(
                AuthorityStage.TOKEN_CONTRACT,
                () -> parseAuthorityTokenResponse(tokenGrant.response())
            );
            atAuthorityStage(
                AuthorityStage.REFRESH_CONTRACT,
                () -> verifyAuthorityRefreshContract(
                    tokens.accessToken(),
                    tokens.refreshToken(),
                    refreshToken,
                    tokens.sessionState(),
                    tokens.refreshExpiresIn(),
                    expectedIssuer,
                    tokenRequestStartedAt,
                    tokenResponseReceivedAt
                )
            );
            BoundedResponse jwks = atAuthorityStage(
                AuthorityStage.JWKS_HTTP,
                () -> sendBounded(
                    client,
                    HttpRequest.newBuilder(
                        URI.create(
                            ADMIN_REALM_URL + "/protocol/openid-connect/certs"
                        )
                    )
                        .timeout(AUTHORITY_REQUEST_TIMEOUT)
                        .header("Accept", "application/json")
                        .GET()
                        .build(),
                    MAX_INPUT_BYTES,
                    proofDeadlineNanos
                )
            );
            atAuthorityStage(AuthorityStage.JWKS_SIGNATURE, () -> {
                CompactJwt idToken = decodeJwt(tokens.idToken());
                verifyIdTokenSignature(
                    idToken,
                    jwks.statusCode(),
                    jwks.body()
                );
            });
            atAuthorityStage(AuthorityStage.TOKEN_CLAIMS, () -> {
                verifyAuthorityTokenResponseTiming(
                    tokens,
                    tokenRequestStartedAt,
                    tokenResponseReceivedAt
                );
                verifyAuthorityTokenPair(
                    tokens.accessToken(),
                    tokens.idToken(),
                    tokens.sessionState(),
                    userId,
                    username,
                    expectedIssuer,
                    Instant.now().getEpochSecond()
                );
            });

            String token = tokens.accessToken();
            atAuthorityStage(AuthorityStage.ACCESSIBLE_REALMS, () -> {
                BoundedResponse response = sendBounded(
                    client,
                    authorisedGet(
                        "http://keycloak:8080/admin/realms"
                            + "?briefRepresentation=true",
                        token
                    ),
                    MAX_INPUT_BYTES,
                    proofDeadlineNanos
                );
                verifyAccessibleRealmsResponse(
                    response.statusCode(),
                    response.body()
                );
            });
            atAuthorityStage(AuthorityStage.TARGET_REALM, () -> {
                BoundedResponse response = sendBounded(
                    client,
                    authorisedGet(
                        "http://keycloak:8080/admin/realms/synveda",
                        token
                    ),
                    MAX_INPUT_BYTES,
                    proofDeadlineNanos
                );
                verifyTargetRealmResponse(response.statusCode(), response.body());
            });
            atAuthorityStage(AuthorityStage.MASTER_INVENTORY, () -> {
                BoundedResponse response = sendBounded(
                    client,
                    authorisedGet(
                        "http://keycloak:8080/admin/realms/master/users"
                            + "?first=0&max=3&briefRepresentation=true&exact=false",
                        token
                    ),
                    32_768,
                    proofDeadlineNanos
                );
                verifyMasterInventoryResponse(
                    response.statusCode(),
                    response.body(),
                    userId,
                    username,
                    expectedBootstrapUserId,
                    bootstrapUsername
                );
            });
            atAuthorityStage(AuthorityStage.MASTER_SELF_QUERY, () -> {
                BoundedResponse response = sendBounded(
                    client,
                    authorisedGet(
                        "http://keycloak:8080/admin/realms/master/users"
                            + "?username="
                            + URLEncoder.encode(
                                username,
                                StandardCharsets.UTF_8
                            )
                            + "&exact=true",
                        token
                    ),
                    16_384,
                    proofDeadlineNanos
                );
                verifyMasterSelfQueryResponse(
                    response.statusCode(),
                    response.body(),
                    userId,
                    username
                );
            });
            String masterSelfUrl =
                "http://keycloak:8080/admin/realms/master/users/" + userId;
            atAuthorityStage(AuthorityStage.MASTER_SELF, () -> {
                BoundedResponse response = sendBounded(
                    client,
                    authorisedGet(masterSelfUrl, token),
                    16_384,
                    proofDeadlineNanos
                );
                verifyMasterSelfResponse(
                    response.statusCode(),
                    response.body(),
                    userId,
                    username
                );
            });
            atAuthorityStage(
                AuthorityStage.MASTER_FEDERATED_IDENTITIES,
                () -> {
                    BoundedResponse response = sendBounded(
                        client,
                        authorisedGet(
                            masterSelfUrl + "/federated-identity",
                            token
                        ),
                        4096,
                        proofDeadlineNanos
                    );
                    verifyEmptyArrayResponse(
                        response.statusCode(),
                        response.body()
                    );
                }
            );
            atAuthorityStage(AuthorityStage.MASTER_CREDENTIALS, () -> {
                BoundedResponse response = sendBounded(
                    client,
                    authorisedGet(masterSelfUrl + "/credentials", token),
                    16_384,
                    proofDeadlineNanos
                );
                verifyPasswordCredentialResponse(
                    response.statusCode(),
                    response.body()
                );
            });
            atAuthorityStage(AuthorityStage.MASTER_CLIENTS, () -> {
                int statusCode = sendDiscarding(
                    client,
                    authorisedGet(
                        "http://keycloak:8080/admin/realms/master/clients"
                            + "?clientId=admin-cli",
                        token
                    ),
                    proofDeadlineNanos
                );
                verifyForbiddenAuthorityResponse(statusCode);
            });
            atAuthorityStage(AuthorityStage.MASTER_SESSION_STATS, () -> {
                int statusCode = sendDiscarding(
                    client,
                    authorisedGet(
                        "http://keycloak:8080/admin/realms/master"
                            + "/client-session-stats",
                        token
                    ),
                    proofDeadlineNanos
                );
                verifyForbiddenAuthorityResponse(statusCode);
            });
            atAuthorityStage(AuthorityStage.DENY_CREATE_REALM, () -> {
                BoundedResponse response = sendBounded(
                    client,
                    authorisedPost(
                        "http://keycloak:8080/admin/realms",
                        token,
                        "{}"
                    ),
                    4096,
                    proofDeadlineNanos
                );
                verifyForbiddenAuthorityResponse(response.statusCode());
            });
            atAuthorityStage(AuthorityStage.DENY_CREATE_MASTER_USER, () -> {
                BoundedResponse response = sendBounded(
                    client,
                    authorisedPost(
                        "http://keycloak:8080/admin/realms/master/users",
                        token,
                        "{}"
                    ),
                    4096,
                    proofDeadlineNanos
                );
                verifyForbiddenAuthorityResponse(response.statusCode());
            });
            atAuthorityStage(AuthorityStage.DENY_UPDATE_MASTER_SELF, () -> {
                BoundedResponse response = sendBounded(
                    client,
                    authorisedPut(
                        masterSelfUrl,
                        token,
                        JSON.writeValueAsString(
                            Map.of(
                                "id", userId,
                                "username", username,
                                "enabled", true
                            )
                        )
                    ),
                    4096,
                    proofDeadlineNanos
                );
                verifyForbiddenAuthorityResponse(response.statusCode());
            });
            atAuthorityStage(AuthorityStage.DENY_ADD_MASTER_REALM_ROLE, () -> {
                BoundedResponse response = sendBounded(
                    client,
                    authorisedPost(
                        masterSelfUrl + "/role-mappings/realm",
                        token,
                        "[]"
                    ),
                    4096,
                    proofDeadlineNanos
                );
                verifyForbiddenAuthorityResponse(response.statusCode());
            });
            atAuthorityStage(
                AuthorityStage.PROOF_DEADLINE,
                () -> requireBeforeDeadline(proofDeadlineNanos)
            );
        }, () -> revokeAndVerifyRefreshRefused(client, refreshToken));
    }

    private static HttpClient adminHttpClient() {
        return HttpClient.newBuilder()
            .connectTimeout(Duration.ofSeconds(2))
            .followRedirects(HttpClient.Redirect.NEVER)
            .version(HttpClient.Version.HTTP_1_1)
            .build();
    }

    private static void requireBeforeDeadline(long deadlineNanos) {
        if (deadlineNanos - System.nanoTime() <= 0) {
            throw new IllegalArgumentException();
        }
    }

    private static HttpRequest formPost(String target, String form) {
        return HttpRequest.newBuilder(URI.create(target))
            .timeout(AUTHORITY_REQUEST_TIMEOUT)
            .header("Accept", "application/json")
            .header("Content-Type", "application/x-www-form-urlencoded")
            .POST(HttpRequest.BodyPublishers.ofString(form, StandardCharsets.UTF_8))
            .build();
    }

    private static JsonNode parseAuthorityTokenResponseBody(
        int statusCode,
        byte[] body
    ) {
        if (statusCode != 200
            || body.length < 2
            || body.length > MAX_TOKEN_RESPONSE_BYTES) {
            throw new IllegalArgumentException();
        }
        JsonNode response;
        try {
            response = JSON.readTree(body);
        } catch (Exception ignored) {
            throw new IllegalArgumentException();
        }
        requireObject(response);
        return response;
    }

    private static AuthorityTokenGrant parseAuthorityTokenGrant(
        int statusCode,
        byte[] body
    ) {
        JsonNode response = parseAuthorityTokenResponseBody(statusCode, body);
        return new AuthorityTokenGrant(
            response,
            compactToken(response.path("refresh_token"))
        );
    }

    static String extractAuthorityRefreshToken(int statusCode, byte[] body) {
        return parseAuthorityTokenGrant(statusCode, body).refreshToken();
    }

    private static AuthorityTokens parseAuthorityTokenResponse(JsonNode response) {
        if (response.size() != 9) {
            throw new IllegalArgumentException();
        }
        text(response, "token_type", "Bearer");
        long expiresIn = integralLong(response.path("expires_in"));
        long refreshExpiresIn = integralLong(
            response.path("refresh_expires_in")
        );
        if (expiresIn <= 0
            || expiresIn > 60
            || refreshExpiresIn <= 0
            || refreshExpiresIn > 1800) {
            throw new IllegalArgumentException();
        }
        number(response, "not-before-policy", 0);
        stringWords(
            response,
            "scope",
            Set.of("email", "openid", "profile")
        );
        String accessToken = compactToken(response.path("access_token"));
        String idToken = compactToken(response.path("id_token"));
        String refreshToken = compactToken(response.path("refresh_token"));
        if (accessToken.equals(idToken)
            || accessToken.equals(refreshToken)
            || idToken.equals(refreshToken)) {
            throw new IllegalArgumentException();
        }
        String sessionState = keycloakSessionState(
            response.path("session_state")
        );
        return new AuthorityTokens(
            accessToken,
            idToken,
            refreshToken,
            sessionState,
            expiresIn,
            refreshExpiresIn
        );
    }

    static void verifyAuthorityTokenResponse(
        int statusCode,
        byte[] body,
        long requestStartedAt,
        long responseReceivedAt
    ) throws Exception {
        AuthorityTokens tokens = parseAuthorityTokenResponse(
            parseAuthorityTokenResponseBody(statusCode, body)
        );
        verifyAuthorityTokenResponseTiming(
            tokens,
            requestStartedAt,
            responseReceivedAt
        );
    }

    private static void verifyAuthorityTokenResponseTiming(
        AuthorityTokens tokens,
        long requestStartedAt,
        long responseReceivedAt
    ) throws Exception {
        verifyAuthorityRequestWindow(requestStartedAt, responseReceivedAt);
        JsonNode claims = decodeJwt(tokens.accessToken()).claims();
        long issuedAt = integralLong(claims.path("iat"));
        long expiresAt = integralLong(claims.path("exp"));
        long lifetime = Math.subtractExact(expiresAt, issuedAt);
        long responseComputedAt = Math.subtractExact(
            expiresAt,
            tokens.expiresIn()
        );
        if (lifetime < 60
            || lifetime > 61
            || responseComputedAt < requestStartedAt
            || responseComputedAt > responseReceivedAt) {
            throw new IllegalArgumentException();
        }
    }

    static void verifyAuthorityRefreshContract(
        String accessToken,
        String refreshToken,
        String expectedRefreshToken,
        String rawSessionState,
        long refreshExpiresIn,
        String expectedIssuer,
        long requestStartedAt,
        long responseReceivedAt
    ) throws Exception {
        String sessionState = exactKeycloakSessionState(rawSessionState);
        verifyAdminIssuer(expectedIssuer);
        if (!refreshToken.equals(expectedRefreshToken)
            || refreshExpiresIn <= 0
            || refreshExpiresIn > 1800) {
            throw new IllegalArgumentException();
        }
        verifyAuthorityRequestWindow(requestStartedAt, responseReceivedAt);

        CompactJwt refresh = decodeJwt(refreshToken);
        JsonNode header = refresh.header();
        requireObject(header);
        if (header.size() != 3) {
            throw new IllegalArgumentException();
        }
        text(header, "alg", "HS512");
        text(header, "typ", "JWT");
        requiredBoundedText(header.path("kid"), 256);

        JsonNode claims = refresh.claims();
        if (claims.size() != 10) {
            throw new IllegalArgumentException();
        }
        text(claims, "typ", "Refresh");
        text(claims, "iss", expectedIssuer);
        text(claims, "aud", expectedIssuer);
        text(claims, "azp", ADMIN_CLIENT);
        text(claims, "sid", sessionState);
        text(claims, "prov", "default");
        exactUuid(requiredText(claims.path("jti")));
        stringWords(
            claims,
            "scope",
            Set.of(
                "acr",
                "basic",
                "email",
                "openid",
                "profile",
                "roles",
                "web-origins"
            )
        );
        JsonNode accessClaims = decodeJwt(accessToken).claims();
        long accessIssuedAt = integralLong(accessClaims.path("iat"));
        long accessExpiresAt = integralLong(accessClaims.path("exp"));
        long issuedAt = integralLong(claims.path("iat"));
        long expiresAt = integralLong(claims.path("exp"));
        long issuedAtDelta = Math.subtractExact(issuedAt, accessIssuedAt);
        long lifetime = Math.subtractExact(expiresAt, issuedAt);
        long responseComputedAt = Math.subtractExact(
            expiresAt,
            refreshExpiresIn
        );
        if (issuedAt < requestStartedAt
            || issuedAt > responseReceivedAt
            || issuedAtDelta < 0
            || issuedAtDelta > 2
            || expiresAt <= issuedAt
            || expiresAt <= accessExpiresAt
            || lifetime != 1800
            || responseComputedAt < issuedAt
            || responseComputedAt > responseReceivedAt) {
            throw new IllegalArgumentException();
        }
    }

    static void verifyAuthorityRequestWindow(
        long requestStartedAt,
        long responseReceivedAt
    ) {
        if (requestStartedAt <= 0
            || responseReceivedAt < requestStartedAt
            || Math.subtractExact(responseReceivedAt, requestStartedAt) > 2) {
            throw new IllegalArgumentException();
        }
    }

    static void verifyAuthorityTokenPair(
        String accessToken,
        String idToken,
        String rawSessionState,
        String rawUserId,
        String username,
        String expectedIssuer,
        long now
    ) throws Exception {
        String sessionState = exactKeycloakSessionState(rawSessionState);
        String userId = exactUuid(rawUserId);
        verifyAdminIssuer(expectedIssuer);
        if (username.isEmpty() || username.length() > 64 || now <= 0) {
            throw new IllegalArgumentException();
        }
        CompactJwt access = decodeJwt(accessToken);
        CompactJwt identity = decodeJwt(idToken);
        JsonNode accessClaims = access.claims();
        JsonNode idClaims = identity.claims();
        if (accessClaims.size() != 8 || idClaims.size() != 13) {
            throw new IllegalArgumentException();
        }
        text(accessClaims, "azp", ADMIN_CLIENT);
        text(accessClaims, "typ", "Bearer");
        text(accessClaims, "iss", expectedIssuer);
        requiredBoundedText(accessClaims.path("jti"), 256);
        text(accessClaims, "sid", sessionState);
        stringWords(
            accessClaims,
            "scope",
            Set.of("email", "openid", "profile")
        );
        TokenTimes accessTimes = verifyFreshTokenTimes(accessClaims, now);
        long accessLifetime = Math.subtractExact(
            accessTimes.expiresAt(),
            accessTimes.issuedAt()
        );
        if (accessLifetime < 60 || accessLifetime > 61) {
            throw new IllegalArgumentException();
        }

        verifyIdHeader(identity.header());
        text(idClaims, "typ", "ID");
        text(idClaims, "acr", "1");
        text(idClaims, "aud", ADMIN_CLIENT);
        text(idClaims, "azp", ADMIN_CLIENT);
        text(idClaims, "iss", expectedIssuer);
        text(idClaims, "sub", userId);
        text(idClaims, "preferred_username", username);
        text(idClaims, "sid", sessionState);
        bool(idClaims, "email_verified", false);
        requiredBoundedText(idClaims.path("jti"), 256);
        TokenTimes idTimes = verifyFreshTokenTimes(idClaims, now);
        long issuedAtDelta = Math.subtractExact(
            idTimes.issuedAt(),
            accessTimes.issuedAt()
        );
        if (idTimes.expiresAt() != accessTimes.expiresAt()
            || issuedAtDelta < 0
            || issuedAtDelta > 2) {
            throw new IllegalArgumentException();
        }
        String actualHash = requiredBoundedText(idClaims.path("at_hash"), 128);
        String expectedHash = accessTokenHash(accessToken);
        if (actualHash.length() != 22
            || !actualHash.matches("[A-Za-z0-9_-]+")
            || !MessageDigest.isEqual(
            actualHash.getBytes(StandardCharsets.US_ASCII),
            expectedHash.getBytes(StandardCharsets.US_ASCII)
        )) {
            throw new IllegalArgumentException();
        }
    }

    private static TokenTimes verifyFreshTokenTimes(JsonNode claims, long now) {
        long issuedAt = integralLong(claims.path("iat"));
        long expiresAt = integralLong(claims.path("exp"));
        if (issuedAt < now - 15
            || issuedAt > now + 5
            || expiresAt <= now) {
            throw new IllegalArgumentException();
        }
        return new TokenTimes(issuedAt, expiresAt);
    }

    static String accessTokenHash(String accessToken) throws Exception {
        byte[] digest = MessageDigest.getInstance("SHA-256").digest(
            compactToken(accessToken).getBytes(StandardCharsets.US_ASCII)
        );
        byte[] leftHalf = java.util.Arrays.copyOf(digest, digest.length / 2);
        return Base64.getUrlEncoder().withoutPadding().encodeToString(leftHalf);
    }

    private static void verifyIdTokenSignature(
        CompactJwt idToken,
        int statusCode,
        byte[] body
    ) throws Exception {
        if (statusCode != 200 || body.length < 2 || body.length > MAX_INPUT_BYTES) {
            throw new IllegalArgumentException();
        }
        verifyIdHeader(idToken.header());
        JsonNode jwks;
        try {
            jwks = JSON.readTree(body);
        } catch (Exception ignored) {
            throw new IllegalArgumentException();
        }
        requireObject(jwks);
        if (jwks.size() != 1) {
            throw new IllegalArgumentException();
        }
        JsonNode keys = jwks.path("keys");
        requireArray(keys);
        String kid = requiredBoundedText(idToken.header().path("kid"), 256);
        List<JsonNode> matches = new ArrayList<>();
        for (JsonNode key : keys) {
            requireObject(key);
            JsonNode candidateKid = key.path("kid");
            if (candidateKid.isTextual() && candidateKid.textValue().equals(kid)) {
                matches.add(key);
            }
        }
        if (matches.size() != 1) {
            throw new IllegalArgumentException();
        }
        JsonNode key = matches.get(0);
        text(key, "kty", "RSA");
        text(key, "alg", "RS256");
        text(key, "use", "sig");
        BigInteger modulus = positiveBase64UrlInteger(key.path("n"));
        BigInteger exponent = positiveBase64UrlInteger(key.path("e"));
        if (modulus.bitLength() < 2048
            || modulus.bitLength() > 8192
            || exponent.compareTo(BigInteger.valueOf(3)) < 0
            || exponent.bitLength() > 32
            || !exponent.testBit(0)) {
            throw new IllegalArgumentException();
        }
        Signature verifier = Signature.getInstance("SHA256withRSA");
        verifier.initVerify(
            KeyFactory.getInstance("RSA").generatePublic(
                new RSAPublicKeySpec(modulus, exponent)
            )
        );
        verifier.update(idToken.signingInput().getBytes(StandardCharsets.US_ASCII));
        if (!verifier.verify(idToken.signature())) {
            throw new IllegalArgumentException();
        }
    }

    static void verifyIdTokenSignature(
        String idToken,
        int statusCode,
        byte[] body
    ) throws Exception {
        verifyIdTokenSignature(decodeJwt(idToken), statusCode, body);
    }

    private static void verifyIdHeader(JsonNode header) {
        requireObject(header);
        if (header.size() != 3) {
            throw new IllegalArgumentException();
        }
        text(header, "alg", "RS256");
        text(header, "typ", "JWT");
        requiredBoundedText(header.path("kid"), 256);
    }

    private static BigInteger positiveBase64UrlInteger(JsonNode node) {
        String value = requiredBoundedText(node, 16_384);
        if (!value.matches("[A-Za-z0-9_-]+")) {
            throw new IllegalArgumentException();
        }
        byte[] decoded = Base64.getUrlDecoder().decode(value);
        if (decoded.length == 0 || decoded[0] == 0) {
            throw new IllegalArgumentException();
        }
        return new BigInteger(1, decoded);
    }

    private static HttpRequest authorisedGet(String target, String token) {
        return HttpRequest.newBuilder(URI.create(target))
            .timeout(AUTHORITY_REQUEST_TIMEOUT)
            .header("Accept", "application/json")
            .header("Authorization", "Bearer " + token)
            .GET()
            .build();
    }

    private static HttpRequest authorisedPost(
        String target,
        String token,
        String body
    ) {
        return HttpRequest.newBuilder(URI.create(target))
            .timeout(AUTHORITY_REQUEST_TIMEOUT)
            .header("Accept", "application/json")
            .header("Authorization", "Bearer " + token)
            .header("Content-Type", "application/json")
            .POST(HttpRequest.BodyPublishers.ofString(body, StandardCharsets.UTF_8))
            .build();
    }

    private static HttpRequest authorisedPut(
        String target,
        String token,
        String body
    ) {
        return HttpRequest.newBuilder(URI.create(target))
            .timeout(AUTHORITY_REQUEST_TIMEOUT)
            .header("Accept", "application/json")
            .header("Authorization", "Bearer " + token)
            .header("Content-Type", "application/json")
            .PUT(HttpRequest.BodyPublishers.ofString(body, StandardCharsets.UTF_8))
            .build();
    }

    static void verifyAccessibleRealmsResponse(int statusCode, byte[] body) {
        if (statusCode != 200
            || body.length < 2
            || body.length > MAX_INPUT_BYTES) {
            throw new IllegalArgumentException();
        }
        JsonNode realms;
        try {
            realms = JSON.readTree(body);
        } catch (Exception ignored) {
            throw new IllegalArgumentException();
        }
        requireArray(realms);
        if (realms.size() != 2) {
            throw new IllegalArgumentException();
        }
        Set<String> realmNames = new HashSet<>();
        for (JsonNode realm : realms) {
            requireObject(realm);
            realmNames.add(requiredText(realm.path("realm")));
        }
        if (!realmNames.equals(Set.of("master", "synveda"))) {
            throw new IllegalArgumentException();
        }
    }

    static void verifyMasterInventoryResponse(
        int statusCode,
        byte[] body,
        String rawUserId,
        String username,
        String rawBootstrapUserId,
        String bootstrapUsername
    ) {
        String userId = exactUuid(rawUserId);
        boolean bootstrapRetired = rawBootstrapUserId.equals("retired");
        String bootstrapUserId = bootstrapRetired
            ? null
            : exactUuid(rawBootstrapUserId);
        if (username.equals(bootstrapUsername)
            || (!bootstrapRetired && userId.equals(bootstrapUserId))
            || statusCode != 200
            || body.length < 2
            || body.length > 32_768) {
            throw new IllegalArgumentException();
        }
        JsonNode users;
        try {
            users = JSON.readTree(body);
        } catch (Exception ignored) {
            throw new IllegalArgumentException();
        }
        requireArray(users);
        if (users.size() != (bootstrapRetired ? 1 : 2)) {
            throw new IllegalArgumentException();
        }
        Map<String, JsonNode> usersById = new HashMap<>();
        for (JsonNode user : users) {
            requireObject(user);
            String id = uuid(user.path("id"));
            if (usersById.put(id, user) != null) {
                throw new IllegalArgumentException();
            }
        }
        verifyMasterUserIdentity(usersById.get(userId), userId, username);
        if (!bootstrapRetired) {
            verifyMasterUserIdentity(
                usersById.get(bootstrapUserId),
                bootstrapUserId,
                bootstrapUsername
            );
        }
    }

    static void verifyMasterSelfQueryResponse(
        int statusCode,
        byte[] body,
        String rawUserId,
        String username
    ) {
        String userId = exactUuid(rawUserId);
        if (statusCode != 200 || body.length < 2 || body.length > 16_384) {
            throw new IllegalArgumentException();
        }
        JsonNode users;
        try {
            users = JSON.readTree(body);
        } catch (Exception ignored) {
            throw new IllegalArgumentException();
        }
        requireArray(users);
        if (users.size() != 1) {
            throw new IllegalArgumentException();
        }
        verifyMasterUserIdentity(users.get(0), userId, username);
    }

    static void verifyMasterSelfResponse(
        int statusCode,
        byte[] body,
        String rawUserId,
        String username
    ) {
        String userId = exactUuid(rawUserId);
        if (statusCode != 200 || body.length < 2 || body.length > 16_384) {
            throw new IllegalArgumentException();
        }
        JsonNode user;
        try {
            user = JSON.readTree(body);
        } catch (Exception ignored) {
            throw new IllegalArgumentException();
        }
        verifyMasterUserIdentity(user, userId, username);
        bool(user, "emailVerified", false);
        JsonNode requiredActions = user.path("requiredActions");
        requireArray(requiredActions);
        if (!requiredActions.isEmpty()) {
            throw new IllegalArgumentException();
        }
        JsonNode attributes = user.path("attributes");
        if (!attributes.isMissingNode() && !attributes.isNull()) {
            requireObject(attributes);
            if (!attributes.isEmpty()) {
                throw new IllegalArgumentException();
            }
        }
    }

    private static void verifyMasterUserIdentity(
        JsonNode user,
        String userId,
        String username
    ) {
        if (user == null) {
            throw new IllegalArgumentException();
        }
        requireObject(user);
        text(user, "id", userId);
        text(user, "username", username);
        bool(user, "enabled", true);
        if (user.hasNonNull("serviceAccountClientId")
            || user.hasNonNull("federationLink")) {
            throw new IllegalArgumentException();
        }
    }

    static void verifyEmptyArrayResponse(int statusCode, byte[] body) {
        if (statusCode != 200 || body.length < 2 || body.length > 4096) {
            throw new IllegalArgumentException();
        }
        JsonNode array;
        try {
            array = JSON.readTree(body);
        } catch (Exception ignored) {
            throw new IllegalArgumentException();
        }
        emptyArray(array);
    }

    static void verifyPasswordCredentialResponse(int statusCode, byte[] body) {
        if (statusCode != 200 || body.length < 2 || body.length > 16_384) {
            throw new IllegalArgumentException();
        }
        JsonNode credentials;
        try {
            credentials = JSON.readTree(body);
        } catch (Exception ignored) {
            throw new IllegalArgumentException();
        }
        requireArray(credentials);
        if (credentials.size() != 1) {
            throw new IllegalArgumentException();
        }
        JsonNode credential = credentials.get(0);
        requireObject(credential);
        uuid(credential.path("id"));
        text(credential, "type", "password");
        JsonNode createdDate = credential.path("createdDate");
        if (!createdDate.isIntegralNumber()
            || !createdDate.canConvertToLong()
            || createdDate.longValue() <= 0
            || credential.hasNonNull("secretData")) {
            throw new IllegalArgumentException();
        }
        JsonNode credentialData = credential.path("credentialData");
        if (!credentialData.isMissingNode() && !credentialData.isNull()) {
            requiredBoundedText(credentialData, 4096);
        }
        JsonNode userLabel = credential.path("userLabel");
        if (!userLabel.isMissingNode() && !userLabel.isNull()) {
            requiredBoundedText(userLabel, 256);
        }
    }

    static void verifyForbiddenAuthorityResponse(int statusCode) {
        if (statusCode != 403) {
            throw new IllegalArgumentException();
        }
    }

    private static void revokeAndVerifyRefreshRefused(
        HttpClient client,
        String refreshToken
    )
        throws Exception {
        long cleanupDeadlineNanos = Math.addExact(
            System.nanoTime(),
            AUTHORITY_CLEANUP_BUDGET.toNanos()
        );
        revokeRefreshToken(client, refreshToken, cleanupDeadlineNanos);

        String refreshForm = "grant_type=refresh_token&client_id=" + ADMIN_CLIENT
            + "&refresh_token="
            + URLEncoder.encode(refreshToken, StandardCharsets.UTF_8);
        BoundedResponse refresh = sendBounded(
            client,
            formPost(ADMIN_REALM_URL + "/protocol/openid-connect/token", refreshForm),
            MAX_TOKEN_RESPONSE_BYTES,
            cleanupDeadlineNanos
        );
        if (refresh.statusCode() == 200) {
            JsonNode response = parseAuthorityTokenResponseBody(
                refresh.statusCode(),
                refresh.body()
            );
            String replacementRefreshToken = compactToken(
                response.path("refresh_token")
            );
            revokeRefreshToken(
                client,
                replacementRefreshToken,
                cleanupDeadlineNanos
            );
            throw new IllegalArgumentException();
        }
        verifyRefreshRefusalResponse(refresh.statusCode(), refresh.body());
    }

    private static void revokeRefreshToken(
        HttpClient client,
        String refreshToken,
        long cleanupDeadlineNanos
    )
        throws Exception {
        String form = "client_id=" + ADMIN_CLIENT
            + "&token="
            + URLEncoder.encode(refreshToken, StandardCharsets.UTF_8)
            + "&token_type_hint=refresh_token";
        BoundedResponse response = sendBounded(
            client,
            formPost(ADMIN_REALM_URL + "/protocol/openid-connect/revoke", form),
            4096,
            cleanupDeadlineNanos
        );
        verifyRevocationResponse(response.statusCode(), response.body());
    }

    static void verifyRevocationResponse(int statusCode, byte[] body) {
        if (statusCode != 200 || body.length > 4096) {
            throw new IllegalArgumentException();
        }
        if (body.length == 0) {
            return;
        }
        JsonNode refusal;
        try {
            refusal = JSON.readTree(body);
        } catch (Exception ignored) {
            throw new IllegalArgumentException();
        }
        requireObject(refusal);
        if (refusal.size() != 2) {
            throw new IllegalArgumentException();
        }
        text(refusal, "error", "invalid_token");
        requiredBoundedText(refusal.path("error_description"), 512);
    }

    static void verifyRefreshRefusalResponse(int statusCode, byte[] body) {
        if (statusCode != 400 || body.length < 2 || body.length > 4096) {
            throw new IllegalArgumentException();
        }
        JsonNode refusal;
        try {
            refusal = JSON.readTree(body);
        } catch (Exception ignored) {
            throw new IllegalArgumentException();
        }
        requireObject(refusal);
        if (refusal.size() != 2) {
            throw new IllegalArgumentException();
        }
        text(refusal, "error", "invalid_grant");
        requiredBoundedText(refusal.path("error_description"), 512);
    }

    private record AuthorityTokens(
        String accessToken,
        String idToken,
        String refreshToken,
        String sessionState,
        long expiresIn,
        long refreshExpiresIn
    ) {}

    private record AuthorityTokenGrant(
        JsonNode response,
        String refreshToken
    ) {}

    private record TokenTimes(long issuedAt, long expiresAt) {}

    private record CompactJwt(
        String raw,
        String signingInput,
        JsonNode header,
        JsonNode claims,
        byte[] signature
    ) {}

    private static void deleteBootstrapUser(JsonNode config, String rawUserId)
        throws Exception {
        requireObject(config);
        String userId = exactUuid(rawUserId);
        String token = accessToken(config);
        HttpClient client = HttpClient.newBuilder()
            .connectTimeout(Duration.ofSeconds(2))
            .followRedirects(HttpClient.Redirect.NEVER)
            .version(HttpClient.Version.HTTP_1_1)
            .build();
        HttpRequest request = HttpRequest.newBuilder(
            URI.create("http://keycloak:8080/admin/realms/master/users/" + userId)
        )
            .timeout(Duration.ofSeconds(4))
            .header("Accept", "application/json")
            .header("Authorization", "Bearer " + token)
            .DELETE()
            .build();
        BoundedResponse response = sendBounded(
            client,
            request,
            0
        );
        verifyBootstrapDeleteResponse(response.statusCode(), response.body());
    }

    static void verifyBootstrapDeleteResponse(int statusCode, byte[] body) {
        if (statusCode != 204 || body.length != 0) {
            throw new IllegalArgumentException();
        }
    }

    static void verifyTargetRealmResponse(int statusCode, byte[] body) {
        if (statusCode != 200 || body.length < 2 || body.length > MAX_INPUT_BYTES) {
            throw new IllegalArgumentException();
        }
        JsonNode realm;
        try {
            realm = JSON.readTree(body);
        } catch (Exception ignored) {
            throw new IllegalArgumentException();
        }
        verifyTargetRealm(realm);
    }

    private static void quarantineTargetRealm(JsonNode config) throws Exception {
        requireObject(config);
        String token = accessToken(config);
        HttpClient client = HttpClient.newBuilder()
            .connectTimeout(Duration.ofSeconds(2))
            .followRedirects(HttpClient.Redirect.NEVER)
            .version(HttpClient.Version.HTTP_1_1)
            .build();
        URI target = URI.create("http://keycloak:8080/admin/realms/synveda");
        HttpRequest update = HttpRequest.newBuilder(target)
            .timeout(Duration.ofSeconds(4))
            .header("Accept", "application/json")
            .header("Authorization", "Bearer " + token)
            .header("Content-Type", "application/json")
            .PUT(HttpRequest.BodyPublishers.ofString(
                "{\"enabled\":false}",
                StandardCharsets.UTF_8
            ))
            .build();
        BoundedResponse updateResponse = sendBounded(client, update, 0);
        HttpRequest readback = HttpRequest.newBuilder(target)
            .timeout(Duration.ofSeconds(4))
            .header("Accept", "application/json")
            .header("Authorization", "Bearer " + token)
            .GET()
            .build();
        BoundedResponse readbackResponse = sendBounded(
            client,
            readback,
            MAX_INPUT_BYTES
        );
        verifyQuarantineResponses(
            updateResponse.statusCode(),
            updateResponse.body(),
            readbackResponse.statusCode(),
            readbackResponse.body()
        );
    }

    static void verifyQuarantineResponses(
        int updateStatusCode,
        byte[] updateBody,
        int readbackStatusCode,
        byte[] readbackBody
    ) {
        if (updateStatusCode != 204
            || updateBody.length != 0
            || readbackStatusCode != 200
            || readbackBody.length < 2
            || readbackBody.length > MAX_INPUT_BYTES) {
            throw new IllegalArgumentException();
        }
        JsonNode realm;
        try {
            realm = JSON.readTree(readbackBody);
        } catch (Exception ignored) {
            throw new IllegalArgumentException();
        }
        verifyRealmState(realm, false);
    }

    private static void verifyBootstrapLoginRefused() throws Exception {
        String username = requiredEnvironment("SYNVEDA_PROBE_USERNAME");
        String password = requiredEnvironment("KC_CLI_PASSWORD");
        String form = "grant_type=password&client_id=admin-cli&username="
            + URLEncoder.encode(username, StandardCharsets.UTF_8)
            + "&password="
            + URLEncoder.encode(password, StandardCharsets.UTF_8);
        BoundedResponse response = sendBounded(
            adminHttpClient(),
            formPost(ADMIN_REALM_URL + "/protocol/openid-connect/token", form),
            4096
        );
        verifyBootstrapRefusalResponse(response.statusCode(), response.body());
    }

    static void verifyBootstrapRefusalResponse(int statusCode, byte[] body) {
        if (statusCode != 400 || body.length < 2 || body.length > 4096) {
            throw new IllegalArgumentException();
        }
        JsonNode refusal;
        try {
            refusal = JSON.readTree(body);
        } catch (Exception ignored) {
            throw new IllegalArgumentException();
        }
        requireObject(refusal);
        if (refusal.size() != 2) {
            throw new IllegalArgumentException();
        }
        text(refusal, "error", "invalid_grant");
        text(refusal, "error_description", "Invalid user credentials");
    }

    private static BoundedResponse sendBounded(
        HttpClient client,
        HttpRequest request,
        int maxBodyBytes
    ) throws Exception {
        long requestTimeoutNanos = request.timeout()
            .orElse(AUTHORITY_REQUEST_TIMEOUT)
            .toNanos();
        return sendBounded(
            client,
            request,
            maxBodyBytes,
            Math.addExact(System.nanoTime(), requestTimeoutNanos)
        );
    }

    private static BoundedResponse sendBounded(
        HttpClient client,
        HttpRequest request,
        int maxBodyBytes,
        long deadlineNanos
    ) throws Exception {
        if (maxBodyBytes < 0) {
            throw new IllegalArgumentException();
        }
        long remainingNanos = deadlineNanos - System.nanoTime();
        long timeoutNanos = Math.min(
            request.timeout().orElse(AUTHORITY_REQUEST_TIMEOUT).toNanos(),
            remainingNanos
        );
        if (timeoutNanos <= 0) {
            throw new IllegalArgumentException();
        }
        CompletableFuture<HttpResponse<byte[]>> pending = client.sendAsync(
            request,
            ignored -> new BoundedBodySubscriber(maxBodyBytes)
        );
        try {
            HttpResponse<byte[]> response = pending.get(
                timeoutNanos,
                TimeUnit.NANOSECONDS
            );
            return new BoundedResponse(response.statusCode(), response.body());
        } catch (InterruptedException interrupted) {
            pending.cancel(true);
            Thread.currentThread().interrupt();
            throw interrupted;
        } catch (ExecutionException | TimeoutException refused) {
            pending.cancel(true);
            throw new IllegalArgumentException();
        }
    }

    private static int sendDiscarding(
        HttpClient client,
        HttpRequest request,
        long deadlineNanos
    ) throws Exception {
        long remainingNanos = deadlineNanos - System.nanoTime();
        long timeoutNanos = Math.min(
            request.timeout().orElse(AUTHORITY_REQUEST_TIMEOUT).toNanos(),
            remainingNanos
        );
        if (timeoutNanos <= 0) {
            throw new IllegalArgumentException();
        }
        CompletableFuture<HttpResponse<Void>> pending = client.sendAsync(
            request,
            HttpResponse.BodyHandlers.discarding()
        );
        try {
            return pending.get(timeoutNanos, TimeUnit.NANOSECONDS).statusCode();
        } catch (InterruptedException interrupted) {
            pending.cancel(true);
            Thread.currentThread().interrupt();
            throw interrupted;
        } catch (ExecutionException | TimeoutException refused) {
            pending.cancel(true);
            throw new IllegalArgumentException();
        }
    }

    private record BoundedResponse(int statusCode, byte[] body) {}

    static final class BoundedBodySubscriber
        implements HttpResponse.BodySubscriber<byte[]> {
        private final int maxBodyBytes;
        private final ByteArrayOutputStream body;
        private final CompletableFuture<byte[]> completion;
        private Flow.Subscription subscription;
        private boolean finished;

        BoundedBodySubscriber(int maxBodyBytes) {
            this.maxBodyBytes = maxBodyBytes;
            this.body = new ByteArrayOutputStream(Math.min(maxBodyBytes, 8192));
            this.completion = new CompletableFuture<>();
        }

        @Override
        public synchronized CompletableFuture<byte[]> getBody() {
            return completion;
        }

        @Override
        public synchronized void onSubscribe(Flow.Subscription next) {
            if (next == null || subscription != null || finished) {
                if (next != null) {
                    next.cancel();
                }
                refuseBody();
                return;
            }
            subscription = next;
            next.request(1);
        }

        @Override
        public synchronized void onNext(List<ByteBuffer> chunks) {
            if (finished) {
                return;
            }
            try {
                for (ByteBuffer chunk : chunks) {
                    int nextSize = Math.addExact(body.size(), chunk.remaining());
                    if (nextSize > maxBodyBytes) {
                        refuseBody();
                        return;
                    }
                    byte[] bytes = new byte[chunk.remaining()];
                    chunk.get(bytes);
                    body.writeBytes(bytes);
                }
                subscription.request(1);
            } catch (Throwable refused) {
                refuseBody();
            }
        }

        @Override
        public synchronized void onError(Throwable ignored) {
            refuseBody();
        }

        @Override
        public synchronized void onComplete() {
            if (finished) {
                return;
            }
            finished = true;
            completion.complete(body.toByteArray());
        }

        private void refuseBody() {
            if (finished) {
                return;
            }
            finished = true;
            if (subscription != null) {
                subscription.cancel();
            }
            completion.completeExceptionally(new IllegalArgumentException());
        }
    }

    private static String requiredEnvironment(String name) {
        String value = System.getenv(name);
        if (value == null || value.isEmpty() || value.length() > 4096) {
            throw new IllegalArgumentException();
        }
        return value;
    }

    private static String accessToken(JsonNode node) {
        List<String> tokens = new ArrayList<>();
        collectAccessTokens(node, tokens);
        if (tokens.size() != 1) {
            throw new IllegalArgumentException();
        }
        return tokens.get(0);
    }

    private static JsonNode decodeAccessToken(JsonNode config) throws Exception {
        return decodeJwt(accessToken(config)).claims();
    }

    private static CompactJwt decodeJwt(String rawToken) throws Exception {
        String token = compactToken(rawToken);
        String[] segments = token.split("\\.", -1);
        byte[] headerBytes = decodeBase64UrlSegment(segments[0], 16_384);
        byte[] claimsBytes = decodeBase64UrlSegment(segments[1], MAX_INPUT_BYTES);
        byte[] signature = decodeBase64UrlSegment(segments[2], 16_384);
        if (headerBytes.length < 2 || claimsBytes.length < 2 || signature.length < 64) {
            throw new IllegalArgumentException();
        }
        JsonNode header;
        JsonNode claims;
        try {
            header = JSON.readTree(headerBytes);
            claims = JSON.readTree(claimsBytes);
        } catch (Exception ignored) {
            throw new IllegalArgumentException();
        }
        requireObject(header);
        requireObject(claims);
        return new CompactJwt(
            token,
            segments[0] + "." + segments[1],
            header,
            claims,
            signature
        );
    }

    private static byte[] decodeBase64UrlSegment(String segment, int maxBytes) {
        if (segment.isEmpty()
            || segment.length() > MAX_TOKEN_BYTES
            || !segment.matches("[A-Za-z0-9_-]+")) {
            throw new IllegalArgumentException();
        }
        byte[] decoded = Base64.getUrlDecoder().decode(segment);
        if (decoded.length > maxBytes
            || !Base64.getUrlEncoder().withoutPadding().encodeToString(decoded)
                .equals(segment)) {
            throw new IllegalArgumentException();
        }
        return decoded;
    }

    private static String compactToken(JsonNode node) {
        return compactToken(requiredText(node));
    }

    private static String compactToken(String token) {
        if (token.isEmpty()
            || token.length() > MAX_TOKEN_BYTES
            || !token.matches("[A-Za-z0-9_-]+\\.[A-Za-z0-9_-]+\\.[A-Za-z0-9_-]+")) {
            throw new IllegalArgumentException();
        }
        return token;
    }

    private static void collectAccessTokens(JsonNode node, List<String> tokens) {
        if (node.isObject()) {
            node.properties().forEach(entry -> {
                if (entry.getKey().equals("token")) {
                    tokens.add(requiredText(entry.getValue()));
                } else {
                    collectAccessTokens(entry.getValue(), tokens);
                }
            });
        } else if (node.isArray()) {
            for (JsonNode child : node) {
                collectAccessTokens(child, tokens);
            }
        }
    }

    private static void exactStringMap(JsonNode node, Map<String, String> expected) {
        requireObject(node);
        if (node.size() != expected.size()) {
            throw new IllegalArgumentException();
        }
        for (Map.Entry<String, String> entry : expected.entrySet()) {
            text(node, entry.getKey(), entry.getValue());
        }
    }

    private static void exactFields(JsonNode node, Set<String> expected) {
        requireObject(node);
        Set<String> actual = new HashSet<>();
        node.fieldNames().forEachRemaining(actual::add);
        if (!actual.equals(expected)) {
            throw new IllegalArgumentException();
        }
    }

    private static void stringSet(JsonNode node, String field, Set<String> expected) {
        JsonNode array = node.path(field);
        requireArray(array);
        Set<String> actual = new HashSet<>();
        for (JsonNode value : array) {
            if (!actual.add(requiredText(value))) {
                throw new IllegalArgumentException();
            }
        }
        if (!actual.equals(expected)) {
            throw new IllegalArgumentException();
        }
    }

    private static void stringWords(JsonNode node, String field, Set<String> expected) {
        String value = requiredText(node.path(field));
        Set<String> actual = new HashSet<>();
        for (String word : value.split(" ", -1)) {
            if (word.isEmpty() || !actual.add(word)) {
                throw new IllegalArgumentException();
            }
        }
        if (!actual.equals(expected)) {
            throw new IllegalArgumentException();
        }
    }

    private static void text(JsonNode node, String field, String expected) {
        if (!requiredText(node.path(field)).equals(expected)) {
            throw new IllegalArgumentException();
        }
    }

    private static String requiredBoundedText(JsonNode node, int maxLength) {
        String value = requiredText(node);
        if (value.length() > maxLength) {
            throw new IllegalArgumentException();
        }
        return value;
    }

    private static String requiredText(JsonNode node) {
        if (!node.isTextual() || node.textValue().isEmpty()) {
            throw new IllegalArgumentException();
        }
        return node.textValue();
    }

    private static String uuid(JsonNode node) {
        String value = requiredText(node);
        return exactUuid(value);
    }

    private static String exactUuid(String value) {
        if (!UUID.matcher(value).matches()) {
            throw new IllegalArgumentException();
        }
        return value;
    }

    private static String keycloakSessionState(JsonNode node) {
        return exactKeycloakSessionState(requiredText(node));
    }

    private static String exactKeycloakSessionState(String value) {
        if (!KEYCLOAK_SESSION_STATE.matcher(value).matches()) {
            throw new IllegalArgumentException();
        }
        return value;
    }

    private static void bool(JsonNode node, String field, boolean expected) {
        JsonNode value = node.path(field);
        if (!value.isBoolean() || value.booleanValue() != expected) {
            throw new IllegalArgumentException();
        }
    }

    private static void boolOrMissingFalse(JsonNode node, String field) {
        JsonNode value = node.path(field);
        if (!value.isMissingNode() && (!value.isBoolean() || value.booleanValue())) {
            throw new IllegalArgumentException();
        }
    }

    private static void number(JsonNode node, String field, long expected) {
        JsonNode value = node.path(field);
        if (!value.isIntegralNumber()
            || !value.canConvertToLong()
            || value.longValue() != expected) {
            throw new IllegalArgumentException();
        }
    }

    private static long integralLong(JsonNode node) {
        if (!node.isIntegralNumber() || !node.canConvertToLong()) {
            throw new IllegalArgumentException();
        }
        return node.longValue();
    }

    private static void verifyAdminIssuer(String raw) throws Exception {
        if (raw.length() > 2048) {
            throw new IllegalArgumentException();
        }
        URI issuer = new URI(raw);
        String scheme = issuer.getScheme();
        int port = issuer.getPort();
        if (!(scheme != null && (scheme.equals("http") || scheme.equals("https")))
            || issuer.getHost() == null
            || issuer.getRawUserInfo() != null
            || !"/realms/master".equals(issuer.getRawPath())
            || issuer.getRawQuery() != null
            || issuer.getRawFragment() != null
            || port == 0
            || port > 65535
            || !raw.equals(issuer.toASCIIString())) {
            throw new IllegalArgumentException();
        }
    }

    private static void requireObject(JsonNode node) {
        if (!node.isObject()) {
            throw new IllegalArgumentException();
        }
    }

    private static void requireArray(JsonNode node) {
        if (!node.isArray()) {
            throw new IllegalArgumentException();
        }
    }
}
