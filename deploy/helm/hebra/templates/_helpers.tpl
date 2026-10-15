{{- define "hebra.fullname" -}}
{{- if contains .Chart.Name .Release.Name -}}
{{- .Release.Name | trunc 50 | trimSuffix "-" -}}
{{- else -}}
{{- printf "%s-%s" .Release.Name .Chart.Name | trunc 50 | trimSuffix "-" -}}
{{- end -}}
{{- end -}}

{{- define "hebra.labels" -}}
app.kubernetes.io/name: {{ .Chart.Name }}
app.kubernetes.io/instance: {{ .Release.Name }}
app.kubernetes.io/version: {{ .Chart.AppVersion | quote }}
app.kubernetes.io/managed-by: {{ .Release.Service }}
helm.sh/chart: {{ printf "%s-%s" .Chart.Name .Chart.Version }}
{{- end -}}

{{/* Selector labels for one component (server or web). Takes (dict "root" $ "component" name). */}}
{{- define "hebra.selector" -}}
app.kubernetes.io/name: {{ .root.Chart.Name }}
app.kubernetes.io/instance: {{ .root.Release.Name }}
app.kubernetes.io/component: {{ .component }}
{{- end -}}

{{- define "hebra.image" -}}
{{- $img := index .root.Values.image .component -}}
{{- if $img.digest -}}
{{ $img.repository }}@{{ $img.digest }}
{{- else -}}
{{ $img.repository }}:{{ $img.tag | default .root.Chart.AppVersion }}
{{- end -}}
{{- end -}}

{{/* The restricted Pod Security Standard, for every container. */}}
{{- define "hebra.containerSecurity" -}}
securityContext:
  runAsNonRoot: true
  runAsUser: 65532
  runAsGroup: 65532
  allowPrivilegeEscalation: false
  readOnlyRootFilesystem: true
  capabilities:
    drop: [ALL]
  seccompProfile:
    type: RuntimeDefault
{{- end -}}

{{- define "hebra.podSecurity" -}}
securityContext:
  runAsNonRoot: true
  runAsUser: 65532
  runAsGroup: 65532
  fsGroup: 65532
  seccompProfile:
    type: RuntimeDefault
automountServiceAccountToken: false
{{- end -}}

{{/* Spread replicas over zones and nodes. */}}
{{- define "hebra.spread" -}}
topologySpreadConstraints:
  - maxSkew: 1
    topologyKey: topology.kubernetes.io/zone
    whenUnsatisfiable: ScheduleAnyway
    labelSelector:
      matchLabels:
        {{- include "hebra.selector" . | nindent 8 }}
  - maxSkew: 1
    topologyKey: kubernetes.io/hostname
    whenUnsatisfiable: ScheduleAnyway
    labelSelector:
      matchLabels:
        {{- include "hebra.selector" . | nindent 8 }}
{{- end -}}

{{- define "hebra.serverScheme" -}}
{{- if .Values.server.tls.enabled }}https{{ else }}http{{ end -}}
{{- end -}}

{{- define "hebra.databaseSecret" -}}
{{- .Values.server.database.existingSecret | default (printf "%s-db-app" (include "hebra.fullname" .)) -}}
{{- end -}}

{{- define "hebra.natsUrl" -}}
{{- .Values.server.nats.url | default (printf "nats://%s-nats:4222" .Release.Name) -}}
{{- end -}}
