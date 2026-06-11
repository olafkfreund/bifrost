{{/* Common naming + labels. */}}

{{- define "bifrost.name" -}}
{{- default .Chart.Name .Values.nameOverride | trunc 63 | trimSuffix "-" -}}
{{- end -}}

{{- define "bifrost.fullname" -}}
{{- .Release.Name | trunc 63 | trimSuffix "-" -}}
{{- end -}}

{{- define "bifrost.labels" -}}
helm.sh/chart: {{ printf "%s-%s" .Chart.Name .Chart.Version | replace "+" "_" | trunc 63 | trimSuffix "-" }}
app.kubernetes.io/name: {{ include "bifrost.name" . }}
app.kubernetes.io/instance: {{ .Release.Name }}
app.kubernetes.io/version: {{ .Chart.AppVersion | quote }}
app.kubernetes.io/managed-by: {{ .Release.Service }}
{{- end -}}

{{- define "bifrost.serviceAccountName" -}}
{{- if .Values.serviceAccount.create -}}
{{- default (include "bifrost.fullname" .) .Values.serviceAccount.name -}}
{{- else -}}
{{- default "default" .Values.serviceAccount.name -}}
{{- end -}}
{{- end -}}

{{/* Image refs default their tag to the chart appVersion. */}}
{{- define "bifrost.apiImage" -}}
{{- printf "%s:%s" .Values.image.api.repository (default .Chart.AppVersion .Values.image.api.tag) -}}
{{- end -}}

{{- define "bifrost.portalImage" -}}
{{- printf "%s:%s" .Values.image.portal.repository (default .Chart.AppVersion .Values.image.portal.tag) -}}
{{- end -}}

{{/* Whether the API uses Postgres (vs SQLite + a PVC). */}}
{{- define "bifrost.usesPostgres" -}}
{{- if or (hasPrefix "postgres://" .Values.api.db) (hasPrefix "postgresql://" .Values.api.db) -}}true{{- else -}}false{{- end -}}
{{- end -}}
