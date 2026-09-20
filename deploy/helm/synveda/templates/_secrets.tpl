{{/* OPS-11: only file paths and existing Secret references enter Pod specs. */}}
{{- define "synveda.databaseTransportEnv" -}}
{{- if eq .Values.postgres.mode "external" -}}
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
{{- if eq .Values.postgres.mode "external" -}}
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
{{- if eq .Values.postgres.mode "external" -}}
- name: postgres-ca
  secret:
    secretName: {{ .Values.postgres.external.caExistingSecret }}
    defaultMode: 0444
    items:
      - key: {{ .Values.postgres.external.caSecretKey }}
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
