{{/* OPS-11: only file paths and existing Secret references enter Pod specs. */}}
{{- define "synveda.databaseTransportEnv" -}}
{{- if eq .Values.postgres.mode "bundled" -}}
- name: SYNVEDA_DATABASE_EXPECTED_HOST
  value: {{ printf "%s-rw" (include "synveda.clusterName" .) | quote }}
- name: SYNVEDA_DATABASE_EXPECTED_PORT
  value: "5432"
- name: SYNVEDA_DATABASE_EXPECTED_NAME
  value: synveda
- name: SYNVEDA_DATABASE_EXPECTED_ROOT_CERT_FILE
  value: /run/secrets/synveda-postgres/ca.crt
{{- else if eq .Values.postgres.mode "external" -}}
- name: SYNVEDA_DATABASE_EXPECTED_HOST
  value: {{ .Values.postgres.external.host | quote }}
- name: SYNVEDA_DATABASE_EXPECTED_PORT
  value: {{ .Values.postgres.external.port | quote }}
- name: SYNVEDA_DATABASE_EXPECTED_NAME
  value: {{ .Values.postgres.external.database | quote }}
- name: SYNVEDA_DATABASE_EXPECTED_ROOT_CERT_FILE
  value: /run/secrets/synveda-postgres/ca.crt
{{- if .Values.postgres.external.clientExistingSecret }}
- name: SYNVEDA_DATABASE_EXPECTED_CLIENT_CERT_FILE
  value: /run/secrets/synveda-postgres-client/tls.crt
- name: SYNVEDA_DATABASE_EXPECTED_CLIENT_KEY_FILE
  value: /run/secrets/synveda-postgres-client/tls.key
{{- end }}
{{- end -}}
{{- end -}}

{{- define "synveda.databaseTransportMounts" -}}
{{- if ne .Values.postgres.mode "cnpg" -}}
- name: postgres-ca
  mountPath: /run/secrets/synveda-postgres
  readOnly: true
{{- if .Values.postgres.external.clientExistingSecret }}
- name: postgres-client
  mountPath: /run/secrets/synveda-postgres-client
  readOnly: true
{{- end }}
{{- end -}}
{{- end -}}

{{- define "synveda.databaseTransportVolumes" -}}
{{- if ne .Values.postgres.mode "cnpg" -}}
- name: postgres-ca
  secret:
    secretName: {{ if eq .Values.postgres.mode "bundled" }}{{ .Values.postgres.bundled.tlsExistingSecret }}{{ else }}{{ .Values.postgres.external.caExistingSecret }}{{ end }}
    defaultMode: 0444
    items:
      - key: {{ if eq .Values.postgres.mode "bundled" }}{{ .Values.postgres.bundled.caSecretKey }}{{ else }}{{ .Values.postgres.external.caSecretKey }}{{ end }}
        path: ca.crt
{{- if .Values.postgres.external.clientExistingSecret }}
- name: postgres-client
  secret:
    secretName: {{ .Values.postgres.external.clientExistingSecret }}
    defaultMode: 0444
    items:
      - key: {{ .Values.postgres.external.clientCertSecretKey }}
        path: tls.crt
      - key: {{ .Values.postgres.external.clientKeySecretKey }}
        path: tls.key
{{- end }}
{{- end -}}
{{- end -}}

{{- define "synveda.runtimeSecretVolume" -}}
- name: runtime-secrets
  projected:
    defaultMode: 0444
    sources:
      - secret:
          name: {{ .Values.oidc.existingSecret }}
          items:
            - key: {{ .Values.oidc.secretKey }}
              path: issuers.json
      - secret:
          name: {{ .Values.kms.existingSecret }}
          items:
            - key: {{ .Values.kms.secretKey }}
              path: kms_key
            - key: {{ .Values.kms.keyRefSecretKey }}
              path: kms_key_ref
{{- end -}}
