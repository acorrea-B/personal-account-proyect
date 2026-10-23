# Variables del deploy (plan §12.2). El project_id es explícito: con keys
# de IAM el provider no lo resuelve desde el token (project_id is required).
# Configuración y no credencial, PERO el repo es público y GitHub solo
# enmascara secrets en los logs — viaja como secrets.SCW_PROJECT_ID.
#
# Las variables no secretas de la función vienen de los `vars` del
# environment de GitHub y conservan los mismos defaults que tenía el
# serverless.yml. Los secrets NUNCA pasan por aquí (plan §12.2): se adjuntan
# fuera del pipeline como secret_environment_variables, y Terraform no los
# declara — no los toca.

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