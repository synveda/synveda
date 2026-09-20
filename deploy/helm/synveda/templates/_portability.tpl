{{/* Omit assigned IDs at the platform seam, including TEI's upstream root UID. */}}
{{- define "synveda.podSecurity" -}}
{{- if eq .root.Values.platform "openshift" -}}
{{- omit .context "runAsUser" "runAsGroup" "fsGroup" "fsGroupChangePolicy" | toYaml -}}
{{- else -}}
{{- toYaml .context -}}
{{- end -}}
{{- end -}}

{{- define "synveda.outboundEnv" -}}
{{- with .Values.outbound.caBundleExistingSecret }}
- name: SSL_CERT_FILE
  value: /run/synveda-ca/ca-bundle.crt
- name: REQUESTS_CA_BUNDLE
  value: /run/synveda-ca/ca-bundle.crt
{{- end }}
{{- if .Values.outbound.proxyExistingSecret }}
{{- range list "HTTP_PROXY" "HTTPS_PROXY" "NO_PROXY" }}
- name: {{ . }}
  valueFrom:
    secretKeyRef:
      name: {{ $.Values.outbound.proxyExistingSecret }}
      key: {{ . }}
{{- end }}
{{- end }}
{{- end -}}

{{- define "synveda.outboundMounts" -}}
{{- if .Values.outbound.caBundleExistingSecret }}
- name: outbound-ca
  mountPath: /run/synveda-ca
  readOnly: true
{{- end }}
{{- end -}}

{{- define "synveda.outboundVolumes" -}}
{{- if .Values.outbound.caBundleExistingSecret }}
- name: outbound-ca
  secret:
    secretName: {{ .Values.outbound.caBundleExistingSecret }}
    items:
      - key: {{ .Values.outbound.caBundleSecretKey }}
        path: ca-bundle.crt
{{- end }}
{{- end -}}
