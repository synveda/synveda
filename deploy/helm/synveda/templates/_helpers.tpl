{{/*
Names, labels, and the checks that refuse a configuration this product
cannot honour. ADR-0062.
*/}}

{{- define "synveda.name" -}}
{{- default .Chart.Name .Values.nameOverride | trunc 63 | trimSuffix "-" -}}
{{- end -}}

{{- define "synveda.fullname" -}}
{{- if .Values.fullnameOverride -}}
{{- .Values.fullnameOverride | trunc 63 | trimSuffix "-" -}}
{{- else -}}
{{- $name := default .Chart.Name .Values.nameOverride -}}
{{- if contains $name .Release.Name -}}
{{- .Release.Name | trunc 63 | trimSuffix "-" -}}
{{- else -}}
{{- printf "%s-%s" .Release.Name $name | trunc 63 | trimSuffix "-" -}}
{{- end -}}
{{- end -}}
{{- end -}}

{{- define "synveda.labels" -}}
helm.sh/chart: {{ printf "%s-%s" .Chart.Name .Chart.Version | replace "+" "_" | trunc 63 | trimSuffix "-" }}
{{ include "synveda.selectorLabels" . }}
app.kubernetes.io/version: {{ .Chart.AppVersion | quote }}
app.kubernetes.io/managed-by: {{ .Release.Service }}
{{- end -}}

{{- define "synveda.selectorLabels" -}}
app.kubernetes.io/name: {{ include "synveda.name" . }}
app.kubernetes.io/instance: {{ .Release.Name }}
{{- end -}}

{{- define "synveda.image" -}}
{{- printf "%s:%s" .Values.image.repository (default .Chart.AppVersion .Values.image.tag) -}}
{{- end -}}

{{- define "synveda.serviceAccountName" -}}
{{- if .Values.serviceAccount.create -}}
{{- default (include "synveda.fullname" .) .Values.serviceAccount.name -}}
{{- else -}}
{{- default "default" .Values.serviceAccount.name -}}
{{- end -}}
{{- end -}}

{{/* The CNPG Cluster and the two secrets it publishes. */}}
{{- define "synveda.clusterName" -}}
{{- printf "%s-pg" (include "synveda.fullname" .) | trunc 63 | trimSuffix "-" -}}
{{- end -}}

{{- define "synveda.appSecret" -}}
{{- printf "%s-app" (include "synveda.clusterName" .) -}}
{{- end -}}

{{- define "synveda.superuserSecret" -}}
{{- printf "%s-superuser" (include "synveda.clusterName" .) -}}
{{- end -}}

{{/*
The application database and the role that reaches it. CNPG owns both
names; they are here so the install job's GRANT and the gateway's DSN
cannot drift apart.
*/}}
{{- define "synveda.dbName" -}}synveda{{- end -}}
{{- define "synveda.appRole" -}}synveda_gateway{{- end -}}

{{/*
The admin identity, for the install job and nothing else. Assembled from
the parts rather than CNPG's own `uri` key, because that one names the
cluster's default database and the schema lives in ours.

Kubernetes expands $(VAR) against earlier entries in the same list, so the
password never appears in a manifest. CNPG generates alphanumeric
passwords, so it needs no URI escaping — if that ever changes, this breaks
loudly at connect time rather than quietly at parse time.
*/}}
{{- define "synveda.adminDsnEnv" -}}
- name: SYNVEDA_PG_ADMIN_USER
  valueFrom:
    secretKeyRef:
      name: {{ include "synveda.superuserSecret" . }}
      key: username
- name: SYNVEDA_PG_ADMIN_PASSWORD
  valueFrom:
    secretKeyRef:
      name: {{ include "synveda.superuserSecret" . }}
      key: password
- name: DATABASE_URL
  value: postgres://$(SYNVEDA_PG_ADMIN_USER):$(SYNVEDA_PG_ADMIN_PASSWORD)@{{ include "synveda.clusterName" . }}-rw:5432/{{ include "synveda.dbName" . }}
{{- end -}}

{{/*
────────────────────────────────────────────────────────────────────────
Refusals. Every one of these is a configuration whose failure would be
silent if the chart rendered it anyway.
────────────────────────────────────────────────────────────────────────
*/}}
{{- define "synveda.validate" -}}

{{- /* Decision 4. A second replica breaks login and serves stale scope
       chains, and both look like something else. OPS-7 lifts this. */ -}}
{{- if or (hasKey .Values.gateway "replicas") (hasKey .Values.gateway "replicaCount") (hasKey .Values "replicaCount") -}}
{{- fail "gateway replicas are not configurable in this chart (ADR-0062 decision 4).\n  Two things in the gateway are process-local and fail silently with more than one replica:\n    - pending logins and CLI handoff codes live in memory (LoginFlow), so an\n      /auth/callback that lands on another pod is a 401 for a login the IdP completed;\n    - policy/scope caches are invalidated in-process, so a scope move handled by one\n      replica can leave another replica deciding against stale ancestry.\n  OPS-7 is the feature that fixes both. Remove the key." -}}
{{- end -}}

{{- /* Decision 6. Origin and redirect URI are both derived from this. */ -}}
{{- if not .Values.gateway.publicUrl -}}
{{- fail "gateway.publicUrl is required: the externally reachable origin of this gateway, e.g. https://synveda.example.com.\n  It is where /auth/callback is registered with your IdP and the Origin console sessions are checked against, so a cluster-internal Service URL here refuses every console session for a reason that reads like a bug." -}}
{{- end -}}
{{- if not (or (hasPrefix "http://" .Values.gateway.publicUrl) (hasPrefix "https://" .Values.gateway.publicUrl)) -}}
{{- fail (printf "gateway.publicUrl must be an absolute http(s) URL, got %q" .Values.gateway.publicUrl) -}}
{{- end -}}
{{- if hasSuffix "/" .Values.gateway.publicUrl -}}
{{- fail (printf "gateway.publicUrl must not end in a slash, got %q — the gateway appends its own paths" .Values.gateway.publicUrl) -}}
{{- end -}}
{{- if and .Values.ingress.enabled .Values.ingress.host -}}
{{- $publicHost := .Values.gateway.publicUrl | trimPrefix "https://" | trimPrefix "http://" | splitList ":" | first -}}
{{- if ne $publicHost .Values.ingress.host -}}
{{- fail (printf "ingress.host (%s) and the host in gateway.publicUrl (%s) disagree. Two settings that must agree are two settings that will not: set one from the other." .Values.ingress.host $publicHost) -}}
{{- end -}}
{{- end -}}
{{- if and .Values.ingress.enabled (not .Values.ingress.host) -}}
{{- fail "ingress.enabled is set but ingress.host is empty" -}}
{{- end -}}

{{- /* Decision 6. One auth mode, never two (ADR-0010) — and the chart
       cannot express the dev one at all. */ -}}
{{- if not .Values.oidc.existingSecret -}}
{{- fail "oidc.existingSecret is required: the name of a Secret holding SYNVEDA_OIDC_ISSUERS.\n  The chart never generates it — an issuer configuration names your directory, and a chart that writes one has invented a trust relationship. Create it with:\n    kubectl create secret generic synveda-oidc --from-file=SYNVEDA_OIDC_ISSUERS=./issuers.json" -}}
{{- end -}}

{{- /* The runtime starts without a KMS only to support local bootstrap and
       diagnostics. A deployed console cannot establish a session that way. */ -}}
{{- if not .Values.kms.existingSecret -}}
{{- fail "kms.existingSecret is required: the name of a Secret holding SYNVEDA_KMS_KEY and SYNVEDA_KMS_KEY_REF.\n  The chart never generates or owns the key. Create it separately, back it up outside the database, and test its restore before relying on encrypted tenant data." -}}
{{- end -}}
{{- if not .Values.kms.secretKey -}}
{{- fail "kms.secretKey must name the Secret key containing the 64-hex-character local KMS key" -}}
{{- end -}}
{{- if not .Values.kms.keyRefSecretKey -}}
{{- fail "kms.keyRefSecretKey must name the Secret key containing the stable KMS key reference" -}}
{{- end -}}

{{- /* Decision 10. The embedder is a property of the corpus. */ -}}
{{- if not .Values.embedder -}}
{{- fail "embedder is required and has no default: `deterministic` or `tei`.\n  Knowledge embedding rows retain model and dimension; a different model converges a separately labelled sidecar rather than reinterpreting old vectors. `deterministic` is lexical-only and must not be labelled semantic." -}}
{{- end -}}
{{- if not (has .Values.embedder (list "deterministic" "tei")) -}}
{{- fail (printf "embedder must be `deterministic` or `tei`, got %q (SUPPORTED_ANN_DIMS is [16, 1024], so a third embedder is a schema question rather than a value)" .Values.embedder) -}}
{{- end -}}
{{- if eq .Values.embedder "tei" -}}
{{- if and (not .Values.tei.enabled) (not .Values.tei.url) -}}
{{- fail "embedder is `tei` but neither tei.enabled nor tei.url is set: run it in-cluster or name an existing endpoint" -}}
{{- end -}}
{{- if and .Values.tei.enabled .Values.tei.url -}}
{{- fail "tei.enabled and tei.url are both set: run TEI in-cluster or point at an existing one, not both" -}}
{{- end -}}
{{- end -}}
{{- if and (ne .Values.embedder "tei") .Values.tei.enabled -}}
{{- fail (printf "tei.enabled is set but embedder is %q, so nothing would use it" .Values.embedder) -}}
{{- end -}}

{{- /* The extractor's own credential, which is not optional for `claude`. */ -}}
{{- if not (has .Values.extractor.kind (list "deterministic" "claude" "vllm")) -}}
{{- fail (printf "extractor.kind must be one of deterministic|claude|vllm, got %q" .Values.extractor.kind) -}}
{{- end -}}
{{- if and (eq .Values.extractor.kind "claude") (not .Values.extractor.existingSecret) -}}
{{- fail "extractor.kind is `claude` but extractor.existingSecret is empty: name a Secret with an ANTHROPIC_API_KEY key" -}}
{{- end -}}
{{- if and (eq .Values.extractor.kind "vllm") (not .Values.extractor.baseUrl) -}}
{{- fail "extractor.kind is `vllm` but extractor.baseUrl is empty" -}}
{{- end -}}
{{- if and (eq .Values.extractor.kind "vllm") (not .Values.extractor.model) -}}
{{- fail "extractor.kind is `vllm` but extractor.model is empty" -}}
{{- end -}}

{{- /* Decision 2's arithmetic, stated where somebody can act on it. */ -}}
{{- if ge (int .Values.gateway.dbMaxConnections) (int .Values.postgres.maxConnections) -}}
{{- fail (printf "gateway.dbMaxConnections (%d) must be below postgres.maxConnections (%d): the gateway's pool is shared by its request handlers and its background loops, and the cluster needs headroom for the operator's own connections" (int .Values.gateway.dbMaxConnections) (int .Values.postgres.maxConnections)) -}}
{{- end -}}

{{- /*
   Decision 1's precondition — the CloudNativePG operator — is deliberately
   *not* checked here. `.Capabilities.APIVersions` is a fabricated set under
   `helm lint` and `helm template`, so a check on it would fail every render
   that has no cluster to ask, and `helm lint` (Helm 4) has no flag to feed
   it one. Helm's own error names the missing kind clearly enough:
   "no matches for kind \"Cluster\" in version postgresql.cnpg.io/v1".
   The requirement is stated in Chart.yaml's annotation, in NOTES.txt and in
   deploy/README.md instead.
*/ -}}

{{- end -}}
