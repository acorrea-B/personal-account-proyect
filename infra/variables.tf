# Variables del deploy (plan §12.2). El proyecto/organización de Scaleway
# llega por el token (SCW_SECRET_KEY): el provider lo resuelve solo.
#
# Las variables no secretas de la función vienen de los `vars` del
# environment de GitHub y conservan los mismos defaults que tenía el
# serverless.yml. Los secrets NUNCA pasan por aquí (plan §12.2): se adjuntan
# fuera del pipeline como secret_environment_variables, y Terraform no los
# declara — no los toca.

variable "stage" {
  description = "Entorno lógico: espacio de nombres separado por entorno."
  type        = string
  validation {
    condition     = contains(["staging", "production"], var.stage)
    error_message = "stage debe ser staging o production."
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