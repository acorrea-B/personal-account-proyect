# Variables del deploy (plan §12.2). El project_id es explícito: con keys
# de IAM el provider no lo resuelve desde el token (project_id is required).
# Configuración y no credencial, PERO el repo es público y GitHub solo
# enmascara secrets en los logs — viaja como secrets.SCW_PROJECT_ID.
#
# REVISIÓN DE D16 (hallazgo del primer deploy): el PUT del provider sobrescribe
# los secret_environment_variables adjuntados a mano fuera del pipeline — la
# consola pierde las variables en cada deploy. Declararlas acá es el plan B
# sancionado por §12.2 ("o Terraform con SOPS" — aquí su versión simple: los
# valores viajan cifrados como secrets de GitHub, nunca en el repo ni en los
# logs; el state remoto guarda los hashes, no los valores).

variable "scw_project_id" {
  description = "Project ID de Scaleway (consola: Project management)."
  type        = string
  validation {
    condition     = can(regex("^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$", var.scw_project_id))
    error_message = "scw_project_id debe ser un UUID."
  }
}

variable "stage" {
  description = "Entorno lógico (solo producción por costos; staging futuro = aditivo)."
  type        = string
  validation {
    condition     = var.stage == "production"
    error_message = "stage debe ser production (sin staging por costos — plan D16)."
  }
}

variable "scw_region" {
  description = "Región de Scaleway."
  type        = string
  default     = "fr-par"
}

variable "allowed_origins" {
  description = "Orígenes CORS permitidos (lista separada por comas)."
  type        = string
  default     = ""
}

variable "max_body_bytes" {
  description = "Tamaño máximo de cuerpo aceptado por la API."
  type        = string
  default     = "65536"
}

variable "request_timeout_secs" {
  description = "Timeout interno de peticiones salientes."
  type        = string
  default     = "10"
}

variable "db_max_connections" {
  description = "Conexiones máximas del pool hacia Supabase."
  type        = string
  default     = "5"
}

variable "rate_limit_login_per_min" {
  description = "Rate limit del endpoint de login por minuto."
  type        = string
  default     = "5"
}

variable "rate_limit_resend_per_10min" {
  description = "Rate limit de reenvío de código por cada 10 minutos."
  type        = string
  default     = "3"
}

variable "rate_limit_cv_write_per_min" {
  description = "Rate limit de escritura del CV por minuto."
  type        = string
  default     = "10"
}

variable "rate_limit_request_per_min" {
  description = "Rate limit global de peticiones por minuto."
  type        = string
  default     = "120"
}

variable "cv_max_bytes" {
  description = "Tamaño máximo del CV subido."
  type        = string
  default     = "51200"
}

variable "cv_versions_retention" {
  description = "Cantidad de versiones del CV que se retienen."
  type        = string
  default     = "10"
}
# --- Credentials of the function (D16 revised: declared, not console-attached) ---

variable "database_url" {
  description = "Postgres of Supabase for the API (transaction pooler, port 6543)."
  type        = string
  sensitive   = true
}

variable "supabase_url" {
  description = "Endpoint of the Supabase project (Auth admin)."
  type        = string
}

variable "supabase_jwks_url" {
  description = "JWKS set of Supabase (ES256 verification; not confidential)."
  type        = string
}

variable "service_role_key" {
  description = "Key with the service role (the admin API, R8)."
  type        = string
  sensitive   = true
}

variable "supabase_publishable_key" {
  description = "Publishable key of the new key system of Supabase."
  type        = string
  sensitive   = true
}

variable "supabase_jwt_issuer" {
  description = "Issuer to enforce in the credentials (cloud: https://<ref>.supabase.co/auth/v1)."
  type        = string
}
