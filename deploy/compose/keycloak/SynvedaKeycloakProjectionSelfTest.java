import java.nio.charset.StandardCharsets;
import java.nio.ByteBuffer;
import java.nio.file.Files;
import java.nio.file.Path;
import java.security.KeyPair;
import java.security.KeyPairGenerator;
import java.security.PrivateKey;
import java.security.Signature;
import java.security.interfaces.RSAPublicKey;
import java.util.Arrays;
import java.util.Base64;
import java.util.List;
import java.util.Map;
import java.util.concurrent.Flow;
import javax.crypto.Mac;
import javax.crypto.spec.SecretKeySpec;

public final class SynvedaKeycloakProjectionSelfTest {
    private SynvedaKeycloakProjectionSelfTest() {}

    public static void main(String[] args) throws Exception {
        Map<Integer, String> authorityStages = Map.ofEntries(
            Map.entry(20, "token-http"),
            Map.entry(21, "token-envelope"),
            Map.entry(22, "jwks-http"),
            Map.entry(23, "jwks-signature"),
            Map.entry(24, "token-claims"),
            Map.entry(25, "accessible-realms"),
            Map.entry(26, "target-realm"),
            Map.entry(27, "master-inventory"),
            Map.entry(28, "master-self-query"),
            Map.entry(29, "master-self"),
            Map.entry(30, "master-federated-identities"),
            Map.entry(31, "master-credentials"),
            Map.entry(32, "master-clients"),
            Map.entry(33, "master-session-stats"),
            Map.entry(34, "deny-create-realm"),
            Map.entry(35, "deny-create-master-user"),
            Map.entry(36, "deny-update-master-self"),
            Map.entry(37, "deny-add-master-realm-role"),
            Map.entry(38, "proof-deadline"),
            Map.entry(39, "cleanup"),
            Map.entry(40, "token-contract"),
            Map.entry(41, "refresh-contract")
        );
        if (!SynvedaKeycloakProjection.authorityStageContract().equals(
            authorityStages
        )) {
            throw new IllegalStateException("authority stage contract drifted");
        }
        byte[] targetRealm = bytes("{\"realm\":\"synveda\"}");
        accept(() -> SynvedaKeycloakProjection.verifyTargetRealmResponse(
            200,
            targetRealm
        ));
        for (int status : new int[] { 0, 201, 204, 400, 401, 403, 404, 429, 500, 503 }) {
            refuse(() -> SynvedaKeycloakProjection.verifyTargetRealmResponse(
                status,
                targetRealm
            ));
        }
        refuse(() -> SynvedaKeycloakProjection.verifyTargetRealmResponse(
            200,
            bytes("{\"realm\":\"master\"}")
        ));
        refuse(() -> SynvedaKeycloakProjection.verifyTargetRealmResponse(
            200,
            bytes("{\"realm\":\"synveda\"} {}")
        ));
        refuse(() -> SynvedaKeycloakProjection.verifyTargetRealmResponse(
            200,
            new byte[1_048_577]
        ));

        byte[] closedRealm = bytes("{\"realm\":\"synveda\",\"enabled\":false}");
        accept(() -> SynvedaKeycloakProjection.verifyQuarantineResponses(
            204,
            new byte[0],
            200,
            closedRealm
        ));
        for (int status : new int[] { 0, 200, 201, 400, 401, 403, 404, 429, 500, 503 }) {
            refuse(() -> SynvedaKeycloakProjection.verifyQuarantineResponses(
                status,
                new byte[0],
                200,
                closedRealm
            ));
        }
        refuse(() -> SynvedaKeycloakProjection.verifyQuarantineResponses(
            204,
            bytes("{}"),
            200,
            closedRealm
        ));
        for (int status : new int[] { 0, 201, 204, 400, 401, 403, 404, 429, 500, 503 }) {
            refuse(() -> SynvedaKeycloakProjection.verifyQuarantineResponses(
                204,
                new byte[0],
                status,
                closedRealm
            ));
        }
        refuse(() -> SynvedaKeycloakProjection.verifyQuarantineResponses(
            204,
            new byte[0],
            200,
            bytes("{\"realm\":\"master\",\"enabled\":false}")
        ));
        refuse(() -> SynvedaKeycloakProjection.verifyQuarantineResponses(
            204,
            new byte[0],
            200,
            bytes("{\"realm\":\"synveda\",\"enabled\":true}")
        ));
        refuse(() -> SynvedaKeycloakProjection.verifyQuarantineResponses(
            204,
            new byte[0],
            200,
            bytes("{\"realm\":\"synveda\",\"enabled\":false} {}")
        ));
        refuse(() -> SynvedaKeycloakProjection.verifyQuarantineResponses(
            204,
            new byte[0],
            200,
            new byte[1_048_577]
        ));

        String demoAdmin = "{\"id\":\"00000000-0000-4000-8000-000000000045\","
            + "\"username\":\"synveda-demo-admin\",\"enabled\":true,"
            + "\"emailVerified\":true,\"email\":\"admin@demo.synveda.invalid\","
            + "\"firstName\":\"Synveda\",\"lastName\":\"Demo Admin\","
            + "\"requiredActions\":[],\"attributes\":{"
            + "\"synvedaDemoContract\":[\"cpr45-demo-v1\"],"
            + "\"synvedaDemoKind\":[\"admin\"]}}";
        accept(() -> SynvedaKeycloakProjection.verifyDemoUser(
            bytes(demoAdmin), "synveda-demo-admin", "admin", true
        ));
        accept(() -> SynvedaKeycloakProjection.verifyDemoUser(
            bytes(demoAdmin.replace("\"enabled\":true", "\"enabled\":false")),
            "synveda-demo-admin", "admin", false
        ));
        refuse(() -> SynvedaKeycloakProjection.verifyDemoUser(
            bytes(demoAdmin.replace("cpr45-demo-v1", "foreign")),
            "synveda-demo-admin", "admin", false
        ));
        refuse(() -> SynvedaKeycloakProjection.verifyDemoUser(
            bytes(demoAdmin.replace("\"enabled\":true", "\"enabled\":false")),
            "synveda-demo-admin", "admin", true
        ));
        refuse(() -> SynvedaKeycloakProjection.verifyDemoUser(
            bytes(demoAdmin.replace(",\"requiredActions\":[]", "")),
            "synveda-demo-admin", "admin", true
        ));
        accept(() -> SynvedaKeycloakProjection.demoUserState(
            bytes(demoAdmin), "synveda-demo-admin", "admin"
        ));
        accept(() -> SynvedaKeycloakProjection.demoUserState(
            bytes("{\"id\":\"00000000-0000-4000-8000-000000000045\","
                + "\"username\":\"synveda-demo-admin\",\"attributes\":{"
                + "\"operatorOwned\":[\"true\"]}}"),
            "synveda-demo-admin", "admin"
        ));
        refuse(() -> SynvedaKeycloakProjection.demoUserState(
            bytes(demoAdmin.replace(
                "\"synvedaDemoKind\":[\"admin\"]",
                "\"unexpected\":[\"admin\"]"
            )),
            "synveda-demo-admin", "admin"
        ));
        byte[] demoGroupMembers = bytes(
            "[{\"id\":\"00000000-0000-4000-8000-000000000045\","
                + "\"username\":\"synveda-demo-admin\",\"enabled\":true}]"
        );
        accept(() -> SynvedaKeycloakProjection.verifyDemoGroupMembers(
            demoGroupMembers,
            "00000000-0000-4000-8000-000000000045",
            "synveda-demo-admin"
        ));
        refuse(() -> SynvedaKeycloakProjection.verifyDemoGroupMembers(
            bytes("[]"),
            "00000000-0000-4000-8000-000000000045",
            "synveda-demo-admin"
        ));
        refuse(() -> SynvedaKeycloakProjection.verifyDemoGroupMembers(
            bytes(new String(demoGroupMembers, StandardCharsets.UTF_8)
                .replace("synveda-demo-admin", "foreign")),
            "00000000-0000-4000-8000-000000000045",
            "synveda-demo-admin"
        ));
        byte[] demoCredential = bytes(
            "[{\"id\":\"00000000-0000-4000-8000-000000000047\","
                + "\"type\":\"password\"}]"
        );
        accept(() -> SynvedaKeycloakProjection.verifyDemoPasswordCredential(
            demoCredential
        ));
        refuse(() -> SynvedaKeycloakProjection.verifyDemoPasswordCredential(
            bytes("[]")
        ));
        refuse(() -> SynvedaKeycloakProjection.verifyDemoPasswordCredential(
            bytes("[{\"id\":\"00000000-0000-4000-8000-000000000047\","
                + "\"type\":\"otp\"}]")
        ));
        accept(() -> SynvedaKeycloakProjection.verifyEmptyRoleMapping(bytes("{}")));
        accept(() -> SynvedaKeycloakProjection.verifyEmptyRoleMapping(
            bytes("{\"realmMappings\":[],\"clientMappings\":{}}")
        ));
        refuse(() -> SynvedaKeycloakProjection.verifyEmptyRoleMapping(
            bytes("{\"realmMappings\":[{\"id\":"
                + "\"00000000-0000-4000-8000-000000000048\"}]}")
        ));

        byte[] userProfile = Files.readAllBytes(
            Path.of("/tmp/synveda-user-profile.json")
        );
        accept(() -> SynvedaKeycloakProjection.verifyUserProfile(userProfile));
        String userProfileJson = new String(userProfile, StandardCharsets.UTF_8);
        refuse(() -> SynvedaKeycloakProjection.verifyUserProfile(bytes(
            userProfileJson.replaceFirst(
                "\\{",
                "{\"unmanagedAttributePolicy\":\"ENABLED\","
            )
        )));
        refuse(() -> SynvedaKeycloakProjection.verifyUserProfile(bytes(
            userProfileJson.replace(
                "\"view\": [\"admin\"],\n        \"edit\": [\"admin\"]",
                "\"view\": [\"admin\", \"user\"],\n"
                    + "        \"edit\": [\"admin\", \"user\"]"
            )
        )));
        refuse(() -> SynvedaKeycloakProjection.verifyUserProfile(bytes(
            userProfileJson.replace(
                "\"options\": {\"options\": [\"cpr45-demo-v1\"]}",
                "\"options\": {\"options\": [\"foreign\"]}"
            )
        )));

        byte[] invalidGrant = bytes(
            "{\"error\":\"invalid_grant\","
                + "\"error_description\":\"Invalid user credentials\"}"
        );
        accept(() -> SynvedaKeycloakProjection.verifyBootstrapRefusalResponse(
            400,
            invalidGrant
        ));
        for (int status : new int[] { 0, 200, 201, 401, 403, 404, 429, 500, 503 }) {
            refuse(() -> SynvedaKeycloakProjection.verifyBootstrapRefusalResponse(
                status,
                invalidGrant
            ));
        }
        refuse(() -> SynvedaKeycloakProjection.verifyBootstrapRefusalResponse(
            400,
            bytes("{\"error\":\"unauthorized_client\"}")
        ));
        refuse(() -> SynvedaKeycloakProjection.verifyBootstrapRefusalResponse(
            400,
            bytes("{\"error\":\"invalid_grant\"}")
        ));
        refuse(() -> SynvedaKeycloakProjection.verifyBootstrapRefusalResponse(
            400,
            bytes(
                "{\"error\":\"invalid_grant\","
                    + "\"error_description\":\"Account disabled\"}"
            )
        ));
        refuse(() -> SynvedaKeycloakProjection.verifyBootstrapRefusalResponse(
            400,
            bytes("{\"error\":\"invalid_grant\",\"error\":\"invalid_grant\"}")
        ));
        refuse(() -> SynvedaKeycloakProjection.verifyBootstrapRefusalResponse(
            400,
            bytes("{\"error\":\"invalid_grant\"} {}")
        ));
        refuse(() -> SynvedaKeycloakProjection.verifyBootstrapRefusalResponse(
            400,
            new byte[4097]
        ));

        String issuer = "http://auth.synveda.test/realms/master";
        String bootstrapId = "00000000-0000-0000-0000-000000000001";
        String bootstrapUsername = "synveda-bootstrap";
        String permanentId = "00000000-0000-0000-0000-000000000002";
        String permanentUsername = "synveda-convergence";
        String sessionId = "Ab0_-Cd1_Ef2-Gh3_Ij4-Kl5";
        String otherSessionId = "Zy9_-Xw8_Vu7-Ts6_Rq5-Po4";
        long now = 2_000_000_000L;
        KeyPairGenerator generator = KeyPairGenerator.getInstance("RSA");
        generator.initialize(2048);
        KeyPair signingKey = generator.generateKeyPair();
        KeyPair wrongKey = generator.generateKeyPair();
        String header = "{\"alg\":\"RS256\",\"kid\":\"fixture-key\",\"typ\":\"JWT\"}";
        String accessPayload = "{\"azp\":\"admin-cli\",\"exp\":" + (now + 60)
            + ",\"iat\":" + now + ",\"iss\":\"" + issuer + "\","
            + "\"jti\":\"access-jti\",\"scope\":\"openid profile email\","
            + "\"sid\":\"" + sessionId + "\",\"typ\":\"Bearer\"}";
        String accessToken = jwt(header, accessPayload, signingKey.getPrivate());
        String configAccessToken = jwt(
            header,
            accessPayload.replace("openid profile email", "email profile"),
            signingKey.getPrivate()
        );
        String refreshHeader =
            "{\"alg\":\"HS512\",\"kid\":\"fixture-internal-key\",\"typ\":\"JWT\"}";
        String refreshPayload = "{\"aud\":\"" + issuer
            + "\",\"azp\":\"admin-cli\",\"exp\":" + (now + 1800)
            + ",\"iat\":" + now + ",\"iss\":\"" + issuer + "\","
            + "\"jti\":\"00000000-0000-0000-0000-000000000005\","
            + "\"prov\":\"default\",\"scope\":"
            + "\"openid web-origins acr roles profile basic email\","
            + "\"sid\":\"" + sessionId + "\",\"typ\":\"Refresh\"}";
        byte[] internalSigningKey = bytes(
            "0123456789abcdef0123456789abcdef"
                + "0123456789abcdef0123456789abcdef"
        );
        String refreshToken = hmacJwt(
            refreshHeader,
            refreshPayload,
            internalSigningKey
        );
        String adminSessionConfig = "{\"serverUrl\":\"http://keycloak:8080\","
            + "\"realm\":\"master\",\"endpoints\":{"
            + "\"http://keycloak:8080\":{\"master\":{"
            + "\"clientId\":\"admin-cli\",\"token\":\"" + configAccessToken + "\","
            + "\"refreshToken\":\"" + refreshToken + "\","
            + "\"grantTypeForAuthentication\":\"password\","
            + "\"expiresAt\":2000000060000,"
            + "\"refreshExpiresAt\":2000001800000}}}}";
        accept(() -> SynvedaKeycloakProjection.verifyAdminSessionConfig(
            bytes(adminSessionConfig)
        ));
        refuse(() -> SynvedaKeycloakProjection.verifyAdminSessionConfig(
            bytes(adminSessionConfig.replace(
                "\"grantTypeForAuthentication\":\"password\"",
                "\"grantTypeForAuthentication\":\"client_credentials\""
            ))
        ));
        refuse(() -> SynvedaKeycloakProjection.verifyAdminSessionConfig(
            bytes(adminSessionConfig.replace(
                ",\"refreshToken\":\"" + refreshToken + "\"",
                ""
            ))
        ));
        refuse(() -> SynvedaKeycloakProjection.verifyAdminSessionConfig(
            bytes(adminSessionConfig.replace(
                "\"refreshToken\":\"" + refreshToken + "\"",
                "\"refreshToken\":\"" + configAccessToken + "\""
            ))
        ));
        refuse(() -> SynvedaKeycloakProjection.verifyAdminSessionConfig(
            bytes(adminSessionConfig.replace(
                "\"realm\":\"master\"",
                "\"realm\":\"master\",\"editor\":\"vi\""
            ))
        ));
        String idPayload = idPayload(
            issuer,
            permanentId,
            sessionId,
            permanentUsername,
            now,
            SynvedaKeycloakProjection.accessTokenHash(accessToken),
            ""
        );
        String idToken = jwt(header, idPayload, signingKey.getPrivate());
        String tokenResponse = tokenResponse(
            accessToken,
            idToken,
            refreshToken,
            sessionId,
            ""
        );
        String extraEnvelope = tokenResponse(
            accessToken,
            idToken,
            refreshToken,
            sessionId,
            ",\"extra\":true"
        );
        expectAuthorityStage(
            () -> SynvedaKeycloakProjection.atAuthorityStage(
                SynvedaKeycloakProjection.AuthorityStage.TOKEN_ENVELOPE,
                () -> {
                    SynvedaKeycloakProjection.extractAuthorityRefreshToken(
                        400,
                        bytes("{\"error\":\"invalid_grant\"}")
                    );
                }
            ),
            SynvedaKeycloakProjection.AuthorityStage.TOKEN_ENVELOPE
        );
        expectAuthorityStage(
            () -> SynvedaKeycloakProjection.atAuthorityStage(
                SynvedaKeycloakProjection.AuthorityStage.TOKEN_ENVELOPE,
                () -> {
                    SynvedaKeycloakProjection.extractAuthorityRefreshToken(
                        200,
                        bytes("{not-json")
                    );
                }
            ),
            SynvedaKeycloakProjection.AuthorityStage.TOKEN_ENVELOPE
        );
        expectAuthorityStage(
            () -> SynvedaKeycloakProjection.atAuthorityStage(
                SynvedaKeycloakProjection.AuthorityStage.TOKEN_ENVELOPE,
                () -> {
                    SynvedaKeycloakProjection.extractAuthorityRefreshToken(
                        200,
                        bytes("{\"refresh_token\":\"not-compact\"}")
                    );
                }
            ),
            SynvedaKeycloakProjection.AuthorityStage.TOKEN_ENVELOPE
        );
        if (!SynvedaKeycloakProjection.extractAuthorityRefreshToken(
            200,
            bytes(extraEnvelope)
        ).equals(refreshToken)) {
            throw new IllegalStateException("refresh cleanup token was not extracted");
        }
        int[] guardedRuns = { 0, 0 };
        expectAuthorityStage(
            () -> SynvedaKeycloakProjection.runAuthorityProofWithCleanup(
                () -> {
                    guardedRuns[0] += 1;
                    SynvedaKeycloakProjection.atAuthorityStage(
                        SynvedaKeycloakProjection.AuthorityStage.TOKEN_CONTRACT,
                        () -> SynvedaKeycloakProjection.verifyAuthorityTokenResponse(
                            200,
                            bytes(extraEnvelope),
                            now,
                            now
                        )
                    );
                },
                () -> {
                    guardedRuns[1] += 1;
                    if (!refreshToken.equals(
                        SynvedaKeycloakProjection.extractAuthorityRefreshToken(
                            200,
                            bytes(extraEnvelope)
                        )
                    )) {
                        throw new IllegalArgumentException();
                    }
                }
            ),
            SynvedaKeycloakProjection.AuthorityStage.TOKEN_CONTRACT
        );
        if (!Arrays.equals(guardedRuns, new int[] { 1, 1 })) {
            throw new IllegalStateException("envelope cleanup guard did not run");
        }
        accept(() -> SynvedaKeycloakProjection.verifyAuthorityRefreshContract(
            accessToken,
            refreshToken,
            refreshToken,
            sessionId,
            1800,
            issuer,
            now,
            now
        ));
        String wrongProviderRefreshToken = hmacJwt(
            refreshHeader,
            refreshPayload.replace("\"prov\":\"default\"", "\"prov\":\"other\""),
            internalSigningKey
        );
        String wrongProviderEnvelope = tokenResponse(
            accessToken,
            idToken,
            wrongProviderRefreshToken,
            sessionId,
            ""
        );
        int[] refreshContractRuns = { 0, 0 };
        expectAuthorityStage(
            () -> SynvedaKeycloakProjection.runAuthorityProofWithCleanup(
                () -> {
                    refreshContractRuns[0] += 1;
                    SynvedaKeycloakProjection.atAuthorityStage(
                        SynvedaKeycloakProjection.AuthorityStage.REFRESH_CONTRACT,
                        () -> SynvedaKeycloakProjection.verifyAuthorityRefreshContract(
                            accessToken,
                            wrongProviderRefreshToken,
                            wrongProviderRefreshToken,
                            sessionId,
                            1800,
                            issuer,
                            now,
                            now
                        )
                    );
                },
                () -> {
                    refreshContractRuns[1] += 1;
                    if (!wrongProviderRefreshToken.equals(
                        SynvedaKeycloakProjection.extractAuthorityRefreshToken(
                            200,
                            bytes(wrongProviderEnvelope)
                        )
                    )) {
                        throw new IllegalArgumentException();
                    }
                }
            ),
            SynvedaKeycloakProjection.AuthorityStage.REFRESH_CONTRACT
        );
        if (!Arrays.equals(refreshContractRuns, new int[] { 1, 1 })) {
            throw new IllegalStateException("refresh-contract cleanup guard did not run");
        }
        expectAuthorityStage(
            () -> SynvedaKeycloakProjection.runAuthorityProofWithCleanup(
                () -> SynvedaKeycloakProjection.atAuthorityStage(
                    SynvedaKeycloakProjection.AuthorityStage.REFRESH_CONTRACT,
                    () -> SynvedaKeycloakProjection.verifyAuthorityRefreshContract(
                        accessToken,
                        wrongProviderRefreshToken,
                        wrongProviderRefreshToken,
                        sessionId,
                        1800,
                        issuer,
                        now,
                        now
                    )
                ),
                () -> {
                    throw new IllegalArgumentException();
                }
            ),
            SynvedaKeycloakProjection.AuthorityStage.CLEANUP
        );
        int[] cleanupOverrideRuns = { 0, 0 };
        expectAuthorityStage(
            () -> SynvedaKeycloakProjection.runAuthorityProofWithCleanup(
                () -> {
                    cleanupOverrideRuns[0] += 1;
                    SynvedaKeycloakProjection.atAuthorityStage(
                        SynvedaKeycloakProjection.AuthorityStage.TOKEN_CLAIMS,
                        () -> {
                            throw new IllegalArgumentException();
                        }
                    );
                },
                () -> {
                    cleanupOverrideRuns[1] += 1;
                    throw new IllegalArgumentException();
                }
            ),
            SynvedaKeycloakProjection.AuthorityStage.CLEANUP
        );
        if (!Arrays.equals(cleanupOverrideRuns, new int[] { 1, 1 })) {
            throw new IllegalStateException("cleanup did not override proof refusal");
        }
        expectAuthorityStage(
            () -> SynvedaKeycloakProjection.runAuthorityProofWithCleanup(
                () -> {},
                () -> SynvedaKeycloakProjection.atAuthorityStage(
                    SynvedaKeycloakProjection.AuthorityStage.JWKS_HTTP,
                    () -> {
                        throw new IllegalArgumentException();
                    }
                )
            ),
            SynvedaKeycloakProjection.AuthorityStage.CLEANUP
        );
        int[] successfulProofCleanupRuns = { 0, 0 };
        expectAuthorityStage(
            () -> SynvedaKeycloakProjection.runAuthorityProofWithCleanup(
                () -> successfulProofCleanupRuns[0] += 1,
                () -> {
                    successfulProofCleanupRuns[1] += 1;
                    throw new IllegalArgumentException();
                }
            ),
            SynvedaKeycloakProjection.AuthorityStage.CLEANUP
        );
        if (!Arrays.equals(successfulProofCleanupRuns, new int[] { 1, 1 })) {
            throw new IllegalStateException("successful proof skipped cleanup");
        }
        int[] successfulGuardRuns = { 0, 0 };
        accept(() -> SynvedaKeycloakProjection.runAuthorityProofWithCleanup(
            () -> successfulGuardRuns[0] += 1,
            () -> successfulGuardRuns[1] += 1
        ));
        if (!Arrays.equals(successfulGuardRuns, new int[] { 1, 1 })) {
            throw new IllegalStateException("successful guard lifecycle drifted");
        }
        for (String scope : new String[] {
            "openid profile email",
            "openid email profile",
            "profile openid email",
            "profile email openid",
            "email openid profile",
            "email profile openid"
        }) {
            String orderedResponse = tokenResponse.replace(
                "openid profile email",
                scope
            );
            accept(() -> SynvedaKeycloakProjection.verifyAuthorityTokenResponse(
                200,
                bytes(orderedResponse),
                now,
                now
            ));
        }
        for (int status : new int[] { 0, 201, 204, 400, 401, 403, 429, 500, 503 }) {
            refuse(() -> SynvedaKeycloakProjection.verifyAuthorityTokenResponse(
                status,
                bytes(tokenResponse),
                now,
                now
            ));
        }
        refuse(() -> SynvedaKeycloakProjection.verifyAuthorityTokenResponse(
            200,
            bytes(tokenResponse(
                accessToken,
                idToken,
                refreshToken,
                sessionId,
                ",\"extra\":true"
            )),
            now,
            now
        ));
        refuse(() -> SynvedaKeycloakProjection.verifyAuthorityTokenResponse(
            200,
            bytes(tokenResponse(accessToken, idToken, accessToken, sessionId, "")),
            now,
            now
        ));
        refuse(() -> SynvedaKeycloakProjection.verifyAuthorityTokenResponse(
            200,
            bytes(tokenResponse(accessToken, idToken, idToken, sessionId, "")),
            now,
            now
        ));
        for (String invalidSessionState : new String[] {
            "",
            "00000000-0000-0000-0000-000000000003",
            "Ab0_-Cd1_Ef2-Gh3_Ij4-Kl",
            "Ab0_-Cd1_Ef2-Gh3_Ij4-Kl56",
            "Ab0_-Cd1_Ef2-Gh3_Ij4-Kl=",
            "Ab0_-Cd1_Ef2-Gh3_Ij4-Kl.",
            "Ab0_-Cd1_Ef2-Gh3_Ij4-Kl ",
            "Ab0_-Cd1_Ef2-Gh3_Ij4-\\n",
            "Ab0_-Cd1_Ef2-Gh3_Ij4-Kl+",
            "Ab0_-Cd1_Ef2-Gh3_Ij4-Kl/"
        }) {
            refuse(() -> SynvedaKeycloakProjection.verifyAuthorityTokenResponse(
                200,
                bytes(tokenResponse(
                    accessToken,
                    idToken,
                    refreshToken,
                    invalidSessionState,
                    ""
                )),
                now,
                now
            ));
        }
        refuse(() -> SynvedaKeycloakProjection.verifyAuthorityTokenResponse(
            200,
            bytes(tokenResponse.replace(
                "\"session_state\":\"" + sessionId + "\"",
                "\"session_state\":7"
            )),
            now,
            now
        ));
        accept(() -> SynvedaKeycloakProjection.verifyAuthorityTokenResponse(
            200,
            bytes(tokenResponse
                .replace("\"expires_in\":60", "\"expires_in\":59")
                .replace("\"refresh_expires_in\":1800", "\"refresh_expires_in\":1799")),
            now,
            now + 1
        ));
        // This parser layer proves only the bounded outer field. Correlation
        // with the refresh JWT makes 1 and 1797 impossible in the combined
        // two-second authority proof and is tested below.
        for (String validRefreshTtl : new String[] { "1", "1797", "1800" }) {
            String boundedRefreshResponse = tokenResponse.replace(
                "\"refresh_expires_in\":1800",
                "\"refresh_expires_in\":" + validRefreshTtl
            );
            accept(() -> SynvedaKeycloakProjection.verifyAuthorityTokenResponse(
                200,
                bytes(boundedRefreshResponse),
                now,
                now
            ));
        }
        for (String invalidScope : new String[] {
            "profile email",
            "openid profile email extra",
            "openid profile profile email",
            "openid  profile email"
        }) {
            String invalidScopeResponse = tokenResponse.replace(
                "openid profile email",
                invalidScope
            );
            refuse(() -> SynvedaKeycloakProjection.verifyAuthorityTokenResponse(
                200,
                bytes(invalidScopeResponse),
                now,
                now
            ));
        }
        for (String invalidTokenResponse : new String[] {
            tokenResponse.replace(
                "\"token_type\":\"Bearer\"",
                "\"token_type\":\"DPoP\""
            ),
            tokenResponse.replace(
                "\"not-before-policy\":0",
                "\"not-before-policy\":1"
            ),
            tokenResponse.replace("\"expires_in\":60", "\"expires_in\":0"),
            tokenResponse.replace("\"expires_in\":60", "\"expires_in\":-1"),
            tokenResponse.replace("\"expires_in\":60", "\"expires_in\":58"),
            tokenResponse.replace("\"expires_in\":60", "\"expires_in\":61"),
            tokenResponse.replace("\"expires_in\":60", "\"expires_in\":60.0"),
            tokenResponse.replace(
                "\"refresh_expires_in\":1800",
                "\"refresh_expires_in\":0"
            ),
            tokenResponse.replace(
                "\"refresh_expires_in\":1800",
                "\"refresh_expires_in\":-1"
            ),
            tokenResponse.replace(
                "\"refresh_expires_in\":1800",
                "\"refresh_expires_in\":1801"
            ),
            tokenResponse.replace(
                "\"refresh_expires_in\":1800",
                "\"refresh_expires_in\":1799.0"
            )
        }) {
            refuse(() -> SynvedaKeycloakProjection.verifyAuthorityTokenResponse(
                200,
                bytes(invalidTokenResponse),
                now,
                now
            ));
        }
        refuse(() -> SynvedaKeycloakProjection.verifyAuthorityTokenResponse(
            200,
            bytes(tokenResponse),
            now + 1,
            now + 1
        ));
        refuse(() -> SynvedaKeycloakProjection.verifyAuthorityTokenResponse(
            200,
            bytes(tokenResponse),
            now,
            now + 3
        ));
        refuse(() -> SynvedaKeycloakProjection.verifyAuthorityTokenResponse(
            200,
            bytes(tokenResponse),
            now + 1,
            now
        ));
        refuse(() -> SynvedaKeycloakProjection.verifyAuthorityTokenResponse(
            200,
            bytes(tokenResponse + " {}"),
            now,
            now
        ));

        accept(() -> SynvedaKeycloakProjection.verifyAuthorityRequestWindow(
            now,
            now
        ));
        accept(() -> SynvedaKeycloakProjection.verifyAuthorityRequestWindow(
            now,
            now + 2
        ));
        for (long[] invalidWindow : new long[][] {
            { 0, 1 },
            { now + 1, now },
            { now, now + 3 }
        }) {
            refuse(() -> SynvedaKeycloakProjection.verifyAuthorityRequestWindow(
                invalidWindow[0],
                invalidWindow[1]
            ));
        }

        for (long responseAt : new long[] { now, now + 1, now + 2 }) {
            long refreshTtl = now + 1800 - responseAt;
            accept(() -> SynvedaKeycloakProjection.verifyAuthorityRefreshContract(
                accessToken,
                refreshToken,
                refreshToken,
                sessionId,
                refreshTtl,
                issuer,
                now,
                responseAt
            ));
        }
        String reorderedRefreshToken = hmacJwt(
            refreshHeader,
            refreshPayload.replace(
                "openid web-origins acr roles profile basic email",
                "email roles openid web-origins profile acr basic"
            ),
            internalSigningKey
        );
        accept(() -> SynvedaKeycloakProjection.verifyAuthorityRefreshContract(
            accessToken,
            reorderedRefreshToken,
            reorderedRefreshToken,
            sessionId,
            1800,
            issuer,
            now,
            now
        ));
        for (String invalidRefreshPayload : new String[] {
            refreshPayload.replace("\"typ\":\"Refresh\"", "\"typ\":\"Bearer\""),
            refreshPayload.replace(
                "\"iss\":\"" + issuer + "\"",
                "\"iss\":\"https://other.example/realms/master\""
            ),
            refreshPayload.replace(
                "\"aud\":\"" + issuer + "\"",
                "\"aud\":\"https://other.example/realms/master\""
            ),
            refreshPayload.replace(
                "\"aud\":\"" + issuer + "\"",
                "\"aud\":[\"" + issuer + "\"]"
            ),
            refreshPayload.replace(
                "\"azp\":\"admin-cli\"",
                "\"azp\":\"other\""
            ),
            refreshPayload.replace(
                "\"sid\":\"" + sessionId + "\"",
                "\"sid\":\"" + otherSessionId + "\""
            ),
            refreshPayload.replace("\"prov\":\"default\"", "\"prov\":\"other\""),
            refreshPayload.replace(
                "00000000-0000-0000-0000-000000000005",
                "not-a-uuid"
            ),
            refreshPayload.replace(
                "\"exp\":" + (now + 1800),
                "\"exp\":" + (now - 1)
            ),
            refreshPayload.replace("\"iat\":" + now, "\"iat\":" + now + ".0"),
            refreshPayload.replace(
                "\"exp\":" + (now + 1800),
                "\"exp\":" + (now + 1800) + ".0"
            ),
            refreshPayload.replace(
                "openid web-origins acr roles profile basic email",
                "openid web-origins acr roles profile basic email offline_access"
            ),
            refreshPayload.replace("web-origins ", ""),
            refreshPayload.replace("roles profile", "roles roles profile"),
            refreshPayload.replace(",\"prov\":\"default\"", ""),
            refreshPayload.replace(
                "\"typ\":\"Refresh\"",
                "\"typ\":\"Refresh\",\"extra\":true"
            ),
            refreshPayload.replace(
                "\"typ\":\"Refresh\"",
                "\"typ\":\"Refresh\",\"typ\":\"Refresh\""
            )
        }) {
            String invalidRefreshToken = hmacJwt(
                refreshHeader,
                invalidRefreshPayload,
                internalSigningKey
            );
            refuse(() -> SynvedaKeycloakProjection.verifyAuthorityRefreshContract(
                accessToken,
                invalidRefreshToken,
                invalidRefreshToken,
                sessionId,
                1800,
                issuer,
                now,
                now
            ));
        }
        for (String invalidRefreshHeader : new String[] {
            refreshHeader.replace("HS512", "RS256"),
            refreshHeader.replace("\"typ\":\"JWT\"", "\"typ\":\"Refresh\""),
            refreshHeader.replace("fixture-internal-key", ""),
            refreshHeader.replace(
                "fixture-internal-key",
                "x".repeat(257)
            ),
            refreshHeader.replace(
                "\"kid\":\"fixture-internal-key\"",
                "\"kid\":7"
            ),
            refreshHeader.replace(",\"kid\":\"fixture-internal-key\"", ""),
            refreshHeader.replace(
                "\"typ\":\"JWT\"",
                "\"typ\":\"JWT\",\"extra\":true"
            )
        }) {
            String invalidRefreshToken = hmacJwt(
                invalidRefreshHeader,
                refreshPayload,
                internalSigningKey
            );
            refuse(() -> SynvedaKeycloakProjection.verifyAuthorityRefreshContract(
                accessToken,
                invalidRefreshToken,
                invalidRefreshToken,
                sessionId,
                1800,
                issuer,
                now,
                now
            ));
        }
        refuse(() -> SynvedaKeycloakProjection.verifyAuthorityRefreshContract(
            accessToken,
            accessToken,
            accessToken,
            sessionId,
            1800,
            issuer,
            now,
            now
        ));
        refuse(() -> SynvedaKeycloakProjection.verifyAuthorityRefreshContract(
            accessToken,
            refreshToken,
            accessToken,
            sessionId,
            1800,
            issuer,
            now,
            now
        ));
        // With an exact 1800-second lifetime, this shorter outer TTL computes
        // a response time after the observed response boundary.
        refuse(() -> SynvedaKeycloakProjection.verifyAuthorityRefreshContract(
            accessToken,
            refreshToken,
            refreshToken,
            sessionId,
            1799,
            issuer,
            now,
            now
        ));
        refuse(() -> SynvedaKeycloakProjection.verifyAuthorityRefreshContract(
            accessToken,
            refreshToken,
            refreshToken,
            otherSessionId,
            1800,
            issuer,
            now,
            now
        ));

        String beforeWindowAccessToken = jwt(
            header,
            retimeJwtPayload(
                accessPayload,
                now,
                now + 60,
                now - 1,
                now + 59
            ),
            signingKey.getPrivate()
        );
        String beforeWindowRefreshToken = hmacJwt(
            refreshHeader,
            retimeJwtPayload(
                refreshPayload,
                now,
                now + 1800,
                now - 1,
                now + 1799
            ),
            internalSigningKey
        );
        refuse(() -> SynvedaKeycloakProjection.verifyAuthorityRefreshContract(
            beforeWindowAccessToken,
            beforeWindowRefreshToken,
            beforeWindowRefreshToken,
            sessionId,
            1799,
            issuer,
            now,
            now
        ));
        String afterWindowAccessToken = jwt(
            header,
            retimeJwtPayload(
                accessPayload,
                now,
                now + 60,
                now + 1,
                now + 61
            ),
            signingKey.getPrivate()
        );
        String afterWindowRefreshToken = hmacJwt(
            refreshHeader,
            retimeJwtPayload(
                refreshPayload,
                now,
                now + 1800,
                now + 1,
                now + 1801
            ),
            internalSigningKey
        );
        refuse(() -> SynvedaKeycloakProjection.verifyAuthorityRefreshContract(
            afterWindowAccessToken,
            afterWindowRefreshToken,
            afterWindowRefreshToken,
            sessionId,
            1800,
            issuer,
            now,
            now
        ));
        refuse(() -> SynvedaKeycloakProjection.verifyAuthorityRefreshContract(
            accessToken,
            beforeWindowRefreshToken,
            beforeWindowRefreshToken,
            sessionId,
            1799,
            issuer,
            now - 1,
            now
        ));
        String lateRefreshToken = hmacJwt(
            refreshHeader,
            retimeJwtPayload(
                refreshPayload,
                now,
                now + 1800,
                now + 3,
                now + 1803
            ),
            internalSigningKey
        );
        refuse(() -> SynvedaKeycloakProjection.verifyAuthorityRefreshContract(
            accessToken,
            lateRefreshToken,
            lateRefreshToken,
            sessionId,
            1800,
            issuer,
            now + 3,
            now + 3
        ));
        String shortRefreshToken = hmacJwt(
            refreshHeader,
            retimeJwtPayload(
                refreshPayload,
                now,
                now + 1800,
                now,
                now + 1799
            ),
            internalSigningKey
        );
        refuse(() -> SynvedaKeycloakProjection.verifyAuthorityRefreshContract(
            accessToken,
            shortRefreshToken,
            shortRefreshToken,
            sessionId,
            1799,
            issuer,
            now,
            now
        ));
        String longRefreshToken = hmacJwt(
            refreshHeader,
            retimeJwtPayload(
                refreshPayload,
                now,
                now + 1800,
                now,
                now + 1801
            ),
            internalSigningKey
        );
        refuse(() -> SynvedaKeycloakProjection.verifyAuthorityRefreshContract(
            accessToken,
            longRefreshToken,
            longRefreshToken,
            sessionId,
            1800,
            issuer,
            now,
            now + 1
        ));
        String equallyLongAccessToken = jwt(
            header,
            retimeJwtPayload(
                accessPayload,
                now,
                now + 60,
                now,
                now + 1800
            ),
            signingKey.getPrivate()
        );
        refuse(() -> SynvedaKeycloakProjection.verifyAuthorityRefreshContract(
            equallyLongAccessToken,
            refreshToken,
            refreshToken,
            sessionId,
            1800,
            issuer,
            now,
            now
        ));
        // A 1799-second token paired with an 1800-second outer TTL computes
        // issuance before iat; exact lifetime independently refuses it too.
        refuse(() -> SynvedaKeycloakProjection.verifyAuthorityRefreshContract(
            accessToken,
            shortRefreshToken,
            shortRefreshToken,
            sessionId,
            1800,
            issuer,
            now,
            now
        ));

        accept(() -> SynvedaKeycloakProjection.verifyAuthorityTokenPair(
            accessToken,
            idToken,
            sessionId,
            permanentId,
            "synveda-convergence",
            issuer,
            now
        ));
        String reorderedAccessToken = jwt(
            header,
            accessPayload.replace(
                "openid profile email",
                "openid email profile"
            ),
            signingKey.getPrivate()
        );
        String reorderedIdToken = jwt(
            header,
            idPayload(
                issuer,
                permanentId,
                sessionId,
                permanentUsername,
                now,
                SynvedaKeycloakProjection.accessTokenHash(reorderedAccessToken),
                ""
            ),
            signingKey.getPrivate()
        );
        accept(() -> SynvedaKeycloakProjection.verifyAuthorityTokenPair(
            reorderedAccessToken,
            reorderedIdToken,
            sessionId,
            permanentId,
            permanentUsername,
            issuer,
            now
        ));
        for (String invalidScope : new String[] {
            "profile email",
            "openid profile email extra",
            "openid profile profile email",
            "openid  profile email"
        }) {
            String invalidScopeAccessToken = jwt(
                header,
                accessPayload.replace("openid profile email", invalidScope),
                signingKey.getPrivate()
            );
            String boundInvalidScopeIdToken = jwt(
                header,
                idPayload(
                    issuer,
                    permanentId,
                    sessionId,
                    permanentUsername,
                    now,
                    SynvedaKeycloakProjection.accessTokenHash(
                        invalidScopeAccessToken
                    ),
                    ""
                ),
                signingKey.getPrivate()
            );
            refuse(() -> SynvedaKeycloakProjection.verifyAuthorityTokenPair(
                invalidScopeAccessToken,
                boundInvalidScopeIdToken,
                sessionId,
                permanentId,
                permanentUsername,
                issuer,
                now
            ));
        }
        for (long idIssuedAt : new long[] { now + 1, now + 2 }) {
            String trailingIdPayload = idPayload.replace(
                "\"iat\":" + now,
                "\"iat\":" + idIssuedAt
            );
            String trailingIdToken = jwt(
                header,
                trailingIdPayload,
                signingKey.getPrivate()
            );
            accept(() -> SynvedaKeycloakProjection.verifyAuthorityTokenPair(
                accessToken,
                trailingIdToken,
                sessionId,
                permanentId,
                permanentUsername,
                issuer,
                now
            ));
        }
        String accessPayload61 = accessPayload.replace(
            "\"exp\":" + (now + 60),
            "\"exp\":" + (now + 61)
        );
        String accessToken61 = jwt(
            header,
            accessPayload61,
            signingKey.getPrivate()
        );
        String idPayload61 = idPayload(
            issuer,
            permanentId,
            sessionId,
            permanentUsername,
            now + 1,
            SynvedaKeycloakProjection.accessTokenHash(accessToken61),
            ""
        );
        String idToken61 = jwt(header, idPayload61, signingKey.getPrivate());
        String tokenResponse61 = tokenResponse(
            accessToken61,
            idToken61,
            refreshToken,
            sessionId,
            ""
        ).replace(
            "\"refresh_expires_in\":1800",
            "\"refresh_expires_in\":1799"
        );
        accept(() -> SynvedaKeycloakProjection.verifyAuthorityTokenResponse(
            200,
            bytes(tokenResponse61),
            now + 1,
            now + 1
        ));
        accept(() -> SynvedaKeycloakProjection.verifyAuthorityTokenPair(
            accessToken61,
            idToken61,
            sessionId,
            permanentId,
            permanentUsername,
            issuer,
            now
        ));
        for (long invalidIdIssuedAt : new long[] { now - 1, now + 3 }) {
            String invalidIdPayload = idPayload.replace(
                "\"iat\":" + now,
                "\"iat\":" + invalidIdIssuedAt
            );
            String invalidIdToken = jwt(
                header,
                invalidIdPayload,
                signingKey.getPrivate()
            );
            refuse(() -> SynvedaKeycloakProjection.verifyAuthorityTokenPair(
                accessToken,
                invalidIdToken,
                sessionId,
                permanentId,
                permanentUsername,
                issuer,
                now
            ));
        }
        String mismatchedExpiryIdToken = jwt(
            header,
            idPayload.replace(
                "\"exp\":" + (now + 60),
                "\"exp\":" + (now + 61)
            ),
            signingKey.getPrivate()
        );
        refuse(() -> SynvedaKeycloakProjection.verifyAuthorityTokenPair(
            accessToken,
            mismatchedExpiryIdToken,
            sessionId,
            permanentId,
            permanentUsername,
            issuer,
            now
        ));
        String shortAccessToken = jwt(
            header,
            accessPayload.replace(
                "\"exp\":" + (now + 60),
                "\"exp\":" + (now + 59)
            ),
            signingKey.getPrivate()
        );
        String shortIdToken = jwt(
            header,
            idPayload(
                issuer,
                permanentId,
                sessionId,
                permanentUsername,
                now,
                SynvedaKeycloakProjection.accessTokenHash(shortAccessToken),
                ""
            ).replace(
                "\"exp\":" + (now + 60),
                "\"exp\":" + (now + 59)
            ),
            signingKey.getPrivate()
        );
        refuse(() -> SynvedaKeycloakProjection.verifyAuthorityTokenPair(
            shortAccessToken,
            shortIdToken,
            sessionId,
            permanentId,
            permanentUsername,
            issuer,
            now
        ));
        String longAccessToken = jwt(
            header,
            accessPayload.replace(
                "\"exp\":" + (now + 60),
                "\"exp\":" + (now + 62)
            ),
            signingKey.getPrivate()
        );
        String longIdToken = jwt(
            header,
            idPayload(
                issuer,
                permanentId,
                sessionId,
                permanentUsername,
                now + 2,
                SynvedaKeycloakProjection.accessTokenHash(longAccessToken),
                ""
            ),
            signingKey.getPrivate()
        );
        refuse(() -> SynvedaKeycloakProjection.verifyAuthorityTokenPair(
            longAccessToken,
            longIdToken,
            sessionId,
            permanentId,
            permanentUsername,
            issuer,
            now
        ));
        for (String badPayload : new String[] {
            idPayload.replace(issuer, "https://other.example/realms/master"),
            idPayload.replace("\"aud\":\"admin-cli\"", "\"aud\":\"other\""),
            idPayload.replace(permanentId, "00000000-0000-0000-0000-000000000004"),
            idPayload.replace("synveda-convergence", "other-user"),
            idPayload.replace("\"at_hash\":\"", "\"at_hash\":\"wrong"),
            idPayload.replace(sessionId, otherSessionId),
            idPayload.replace("\"iat\":" + now, "\"iat\":" + (now - 30)),
            idPayload.replace("\"iss\":\"" + issuer + "\"", "\"iss\":[\"" + issuer + "\"]"),
            idPayload.replace("\"typ\":\"ID\"", "\"typ\":\"ID\",\"extra\":true"),
            idPayload.replace(",\"email_verified\":false", ""),
            idPayload.replace("\"typ\":\"ID\"", "\"typ\":\"ID\",\"typ\":\"ID\"")
        }) {
            String badToken = jwt(header, badPayload, signingKey.getPrivate());
            refuse(() -> SynvedaKeycloakProjection.verifyAuthorityTokenPair(
                accessToken,
                badToken,
                sessionId,
                permanentId,
                "synveda-convergence",
                issuer,
                now
            ));
        }

        String jwks = jwks((RSAPublicKey) signingKey.getPublic(), "fixture-key", "");
        accept(() -> SynvedaKeycloakProjection.verifyIdTokenSignature(
            idToken,
            200,
            bytes(jwks)
        ));
        String wrongSignature = jwt(header, idPayload, wrongKey.getPrivate());
        refuse(() -> SynvedaKeycloakProjection.verifyIdTokenSignature(
            wrongSignature,
            200,
            bytes(jwks)
        ));
        String wrongAlgorithm = jwt(
            header.replace("RS256", "HS256"),
            idPayload,
            signingKey.getPrivate()
        );
        refuse(() -> SynvedaKeycloakProjection.verifyIdTokenSignature(
            wrongAlgorithm,
            200,
            bytes(jwks)
        ));
        String wrongKid = jwt(
            header.replace("fixture-key", "other-key"),
            idPayload,
            signingKey.getPrivate()
        );
        refuse(() -> SynvedaKeycloakProjection.verifyIdTokenSignature(
            wrongKid,
            200,
            bytes(jwks)
        ));
        refuse(() -> SynvedaKeycloakProjection.verifyIdTokenSignature(
            idToken,
            200,
            bytes(jwks.replace("\"kty\":\"RSA\"", "\"kty\":\"EC\""))
        ));
        refuse(() -> SynvedaKeycloakProjection.verifyIdTokenSignature(
            idToken,
            200,
            bytes(jwks((RSAPublicKey) signingKey.getPublic(), "fixture-key", ",\"kid\":\"fixture-key\""))
        ));
        refuse(() -> SynvedaKeycloakProjection.verifyIdTokenSignature(
            idToken,
            200,
            bytes(jwks + " {}")
        ));

        byte[] accessibleRealms = bytes(
            "[{\"realm\":\"master\"},{\"realm\":\"synveda\"}]"
        );
        accept(() -> SynvedaKeycloakProjection.verifyAccessibleRealmsResponse(
            200,
            accessibleRealms
        ));
        refuse(() -> SynvedaKeycloakProjection.verifyAccessibleRealmsResponse(
            200,
            bytes("[{\"realm\":\"master\"},{\"realm\":\"other\"}]")
        ));
        refuse(() -> SynvedaKeycloakProjection.verifyAccessibleRealmsResponse(
            403,
            accessibleRealms
        ));

        byte[] permanentBrief = bytes(
            "[{\"id\":\"" + permanentId + "\",\"username\":\""
                + permanentUsername + "\",\"enabled\":true}]"
        );
        byte[] activeInventory = bytes(
            "[{\"id\":\"" + bootstrapId + "\",\"username\":\""
                + bootstrapUsername + "\",\"enabled\":true},"
                + "{\"id\":\"" + permanentId + "\",\"username\":\""
                + permanentUsername + "\",\"enabled\":true}]"
        );
        accept(() -> SynvedaKeycloakProjection.verifyMasterInventoryResponse(
            200,
            activeInventory,
            permanentId,
            permanentUsername,
            bootstrapId,
            bootstrapUsername
        ));
        accept(() -> SynvedaKeycloakProjection.verifyMasterInventoryResponse(
            200,
            permanentBrief,
            permanentId,
            permanentUsername,
            "retired",
            bootstrapUsername
        ));
        refuse(() -> SynvedaKeycloakProjection.verifyMasterInventoryResponse(
            200,
            activeInventory,
            permanentId,
            permanentUsername,
            "retired",
            bootstrapUsername
        ));
        refuse(() -> SynvedaKeycloakProjection.verifyMasterInventoryResponse(
            200,
            bytes(
                new String(activeInventory, StandardCharsets.UTF_8)
                    .replace(
                        "]",
                        ",{\"id\":\"00000000-0000-0000-0000-000000000099\","
                            + "\"username\":\"service-account-extra\","
                            + "\"enabled\":true}]"
                    )
            ),
            permanentId,
            permanentUsername,
            bootstrapId,
            bootstrapUsername
        ));

        accept(() -> SynvedaKeycloakProjection.verifyMasterSelfQueryResponse(
            200,
            permanentBrief,
            permanentId,
            permanentUsername
        ));
        refuse(() -> SynvedaKeycloakProjection.verifyMasterSelfQueryResponse(
            200,
            bytes("[]"),
            permanentId,
            permanentUsername
        ));
        byte[] masterSelf = bytes(
            "{\"id\":\"" + permanentId + "\",\"username\":\""
                + permanentUsername + "\",\"enabled\":true,"
                + "\"emailVerified\":false,\"requiredActions\":[],"
                + "\"attributes\":{}}"
        );
        accept(() -> SynvedaKeycloakProjection.verifyMasterSelfResponse(
            200,
            masterSelf,
            permanentId,
            permanentUsername
        ));
        refuse(() -> SynvedaKeycloakProjection.verifyMasterSelfResponse(
            200,
            bytes(new String(masterSelf, StandardCharsets.UTF_8)
                .replace("\"requiredActions\":[]", "\"requiredActions\":[\"UPDATE_PASSWORD\"]")),
            permanentId,
            permanentUsername
        ));

        accept(() -> SynvedaKeycloakProjection.verifyEmptyArrayResponse(
            200,
            bytes("[]")
        ));
        refuse(() -> SynvedaKeycloakProjection.verifyEmptyArrayResponse(
            200,
            bytes("[{}]")
        ));
        byte[] passwordCredential = bytes(
            "[{\"id\":\"00000000-0000-0000-0000-000000000030\","
                + "\"type\":\"password\",\"createdDate\":1}]"
        );
        accept(() -> SynvedaKeycloakProjection.verifyPasswordCredentialResponse(
            200,
            passwordCredential
        ));
        refuse(() -> SynvedaKeycloakProjection.verifyPasswordCredentialResponse(
            200,
            bytes(new String(passwordCredential, StandardCharsets.UTF_8)
                .replace("\"password\"", "\"otp\""))
        ));
        refuse(() -> SynvedaKeycloakProjection.verifyPasswordCredentialResponse(
            200,
            bytes(
                "[{\"id\":\"00000000-0000-0000-0000-000000000030\","
                    + "\"type\":\"password\",\"createdDate\":1,"
                    + "\"secretData\":\"forbidden\"}]"
            )
        ));

        SynvedaKeycloakProjection.verifyForbiddenAuthorityResponse(403);
        for (int status : new int[] { 0, 200, 201, 204, 400, 401, 404, 429, 500, 503 }) {
            refuse(() -> SynvedaKeycloakProjection.verifyForbiddenAuthorityResponse(
                status
            ));
        }

        accept(() -> SynvedaKeycloakProjection.verifyRevocationResponse(
            200,
            new byte[0]
        ));
        accept(() -> SynvedaKeycloakProjection.verifyRevocationResponse(
            200,
            bytes("{\"error\":\"invalid_token\",\"error_description\":\"Invalid token\"}")
        ));
        for (int status : new int[] { 0, 201, 204, 400, 401, 403, 429, 500, 503 }) {
            refuse(() -> SynvedaKeycloakProjection.verifyRevocationResponse(
                status,
                new byte[0]
            ));
        }
        refuse(() -> SynvedaKeycloakProjection.verifyRevocationResponse(
            200,
            bytes("{}")
        ));
        refuse(() -> SynvedaKeycloakProjection.verifyRevocationResponse(
            200,
            bytes("{\"error\":\"invalid_grant\",\"error_description\":\"refused\"}")
        ));
        accept(() -> SynvedaKeycloakProjection.verifyRefreshRefusalResponse(
            400,
            bytes("{\"error\":\"invalid_grant\",\"error_description\":\"Session not active\"}")
        ));
        for (int status : new int[] { 0, 200, 201, 204, 401, 403, 429, 500, 503 }) {
            refuse(() -> SynvedaKeycloakProjection.verifyRefreshRefusalResponse(
                status,
                bytes("{\"error\":\"invalid_grant\",\"error_description\":\"refused\"}")
            ));
        }
        refuse(() -> SynvedaKeycloakProjection.verifyRefreshRefusalResponse(
            400,
            bytes("{\"error\":\"invalid_grant\",\"error_description\":\"refused\",\"access_token\":\"unexpected\"}")
        ));
        refuse(() -> SynvedaKeycloakProjection.verifyRefreshRefusalResponse(
            400,
            bytes("{\"error\":\"temporarily_unavailable\",\"error_description\":\"refused\"}")
        ));

        CountingSubscription boundedSubscription = new CountingSubscription();
        SynvedaKeycloakProjection.BoundedBodySubscriber boundedBody =
            new SynvedaKeycloakProjection.BoundedBodySubscriber(4);
        boundedBody.onSubscribe(boundedSubscription);
        boundedBody.onNext(List.of(ByteBuffer.wrap(bytes("ab"))));
        if (boundedBody.getBody().isDone() || boundedSubscription.requests != 2) {
            throw new IllegalStateException("bounded body completed before EOF");
        }
        boundedBody.onNext(List.of(ByteBuffer.wrap(bytes("cd"))));
        boundedBody.onComplete();
        if (!Arrays.equals(boundedBody.getBody().join(), bytes("abcd"))
            || boundedSubscription.cancelled) {
            throw new IllegalStateException("bounded body was not read exactly");
        }

        CountingSubscription oversizedSubscription = new CountingSubscription();
        SynvedaKeycloakProjection.BoundedBodySubscriber oversizedBody =
            new SynvedaKeycloakProjection.BoundedBodySubscriber(3);
        oversizedBody.onSubscribe(oversizedSubscription);
        oversizedBody.onNext(List.of(ByteBuffer.wrap(bytes("abcd"))));
        if (!oversizedBody.getBody().isCompletedExceptionally()
            || !oversizedSubscription.cancelled) {
            throw new IllegalStateException("oversized body was not cancelled");
        }

        accept(() -> SynvedaKeycloakProjection.verifyBootstrapDeleteResponse(
            204,
            new byte[0]
        ));
        for (int status : new int[] { 0, 200, 201, 400, 401, 403, 404, 429, 500, 503 }) {
            refuse(() -> SynvedaKeycloakProjection.verifyBootstrapDeleteResponse(
                status,
                new byte[0]
            ));
        }
        refuse(() -> SynvedaKeycloakProjection.verifyBootstrapDeleteResponse(
            204,
            bytes("{}")
        ));
        System.out.println("Keycloak authority response self-test passed");
    }

    private static byte[] bytes(String value) {
        return value.getBytes(StandardCharsets.UTF_8);
    }

    private static String tokenResponse(
        String accessToken,
        String idToken,
        String refreshToken,
        String sessionId,
        String extra
    ) {
        return "{\"access_token\":\"" + accessToken + "\",\"expires_in\":60,"
            + "\"id_token\":\"" + idToken + "\",\"not-before-policy\":0,"
            + "\"refresh_expires_in\":1800,\"refresh_token\":\"" + refreshToken + "\","
            + "\"scope\":\"openid profile email\",\"session_state\":\""
            + sessionId + "\",\"token_type\":\"Bearer\"" + extra + "}";
    }

    private static String idPayload(
        String issuer,
        String userId,
        String sessionId,
        String username,
        long issuedAt,
        String accessHash,
        String extra
    ) {
        return "{\"acr\":\"1\",\"at_hash\":\"" + accessHash
            + "\",\"aud\":\"admin-cli\",\"azp\":\"admin-cli\","
            + "\"email_verified\":false,\"exp\":" + (issuedAt + 60)
            + ",\"iat\":" + issuedAt + ",\"iss\":\"" + issuer + "\","
            + "\"jti\":\"id-jti\",\"preferred_username\":\"" + username + "\","
            + "\"sid\":\"" + sessionId + "\",\"sub\":\"" + userId
            + "\",\"typ\":\"ID\"" + extra + "}";
    }

    private static String retimeJwtPayload(
        String payload,
        long oldIssuedAt,
        long oldExpiresAt,
        long newIssuedAt,
        long newExpiresAt
    ) {
        String oldIssuedAtField = "\"iat\":" + oldIssuedAt;
        String oldExpiresAtField = "\"exp\":" + oldExpiresAt;
        if (payload.indexOf(oldIssuedAtField) < 0
            || payload.indexOf(oldIssuedAtField) != payload.lastIndexOf(oldIssuedAtField)
            || payload.indexOf(oldExpiresAtField) < 0
            || payload.indexOf(oldExpiresAtField) != payload.lastIndexOf(oldExpiresAtField)) {
            throw new IllegalArgumentException("fixture timing field drifted");
        }
        return payload
            .replace(oldIssuedAtField, "\"iat\":" + newIssuedAt)
            .replace(oldExpiresAtField, "\"exp\":" + newExpiresAt);
    }

    private static String jwt(String header, String payload, PrivateKey key)
        throws Exception {
        Base64.Encoder encoder = Base64.getUrlEncoder().withoutPadding();
        String signingInput = encoder.encodeToString(bytes(header)) + "."
            + encoder.encodeToString(bytes(payload));
        Signature signer = Signature.getInstance("SHA256withRSA");
        signer.initSign(key);
        signer.update(signingInput.getBytes(StandardCharsets.US_ASCII));
        return signingInput + "." + encoder.encodeToString(signer.sign());
    }

    private static String hmacJwt(String header, String payload, byte[] key)
        throws Exception {
        Base64.Encoder encoder = Base64.getUrlEncoder().withoutPadding();
        String signingInput = encoder.encodeToString(bytes(header)) + "."
            + encoder.encodeToString(bytes(payload));
        Mac signer = Mac.getInstance("HmacSHA512");
        signer.init(new SecretKeySpec(key, "HmacSHA512"));
        return signingInput + "." + encoder.encodeToString(
            signer.doFinal(signingInput.getBytes(StandardCharsets.US_ASCII))
        );
    }

    private static String jwks(RSAPublicKey key, String kid, String extra) {
        Base64.Encoder encoder = Base64.getUrlEncoder().withoutPadding();
        String modulus = encoder.encodeToString(unsigned(key.getModulus().toByteArray()));
        String exponent = encoder.encodeToString(unsigned(key.getPublicExponent().toByteArray()));
        return "{\"keys\":[{\"kid\":\"" + kid + "\",\"kty\":\"RSA\","
            + "\"alg\":\"RS256\",\"use\":\"sig\",\"n\":\"" + modulus
            + "\",\"e\":\"" + exponent + "\"" + extra + "}]}";
    }

    private static byte[] unsigned(byte[] value) {
        return value.length > 1 && value[0] == 0
            ? Arrays.copyOfRange(value, 1, value.length)
            : value;
    }

    private static final class CountingSubscription
        implements Flow.Subscription {
        private int requests;
        private boolean cancelled;

        @Override
        public void request(long count) {
            if (count != 1 || cancelled) {
                throw new IllegalStateException("invalid body demand");
            }
            requests += 1;
        }

        @Override
        public void cancel() {
            cancelled = true;
        }
    }

    private static void accept(CheckedRunnable action) throws Exception {
        action.run();
    }

    private static void refuse(CheckedRunnable action) throws Exception {
        try {
            action.run();
        } catch (IllegalArgumentException expected) {
            return;
        }
        throw new IllegalStateException("authority response was accepted");
    }

    private static void expectAuthorityStage(
        CheckedRunnable action,
        SynvedaKeycloakProjection.AuthorityStage expectedStage
    ) throws Exception {
        try {
            action.run();
        } catch (SynvedaKeycloakProjection.AuthorityProofRefusal refused) {
            if (refused.stage() == expectedStage) {
                return;
            }
            throw new IllegalStateException("authority refusal stage drifted");
        }
        throw new IllegalStateException("authority stage refusal was accepted");
    }

    @FunctionalInterface
    private interface CheckedRunnable {
        void run() throws Exception;
    }
}
