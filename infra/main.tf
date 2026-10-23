# Infraestructura de la API `perfil` en Scaleway Serverless Functions
# (plan §12.2 — deploy por tags). Única fuente de verdad del deploy: reemplaza
# al Serverless Framework (osls/serverless.yml, eliminado).
#
# La plataforma compila el crate: Terraform solo sube el zip de las fuentes
# (generado antes del apply por ../package.sh) y cambia `zip_hash` gatilla el
# re-deploy de la función.

terraform {
  required_version = ">= 1.10"

  required_providers {
    scaleway = {
      source  = "scaleway/scaleway"
      version = "~> 2.50"
    }
  }
}

provider "scaleway" {
  region     = var.scw_region
  project_id = var.scw_project_id
  # project_id explícito: con keys de IAM el provider no lo resuelve solo
  # desde el token (`invalid argument(s): project_id is required`).
}

locals {
  # Variables no secretas de la función (el namespace las hereda). Las
  # credenciales van declaradas aparte, abajo, como secret_environment_variables.
  function_environment = {
    ENV                          = var.stage
    ALLOWED_ORIGINS              = var.allowed_origins
    MAX_BODY_BYTES               = var.max_body_bytes
    REQUEST_TIMEOUT_SECS         = var.request_timeout_secs
    DB_MAX_CONNECTIONS           = var.db_max_connections
    RATE_LIMIT_LOGIN_PER_MIN     = var.rate_limit_login_per_min
    RATE_LIMIT_RESEND_PER_10MIN  = var.rate_limit_resend_per_10min
    RATE_LIMIT_CV_WRITE_PER_MIN  = var.rate_limit_cv_write_per_min
    RATE_LIMIT_REQUEST_PER_MIN   = var.rate_limit_request_per_min
    CV_MAX_BYTES                 = var.cv_max_bytes
    CV_VERSIONS_RETENTION        = var.cv_versions_retention
    SUPABASE_URL                 = var.supabase_url
    SUPABASE_JWT_ISSUER          = var.supabase_jwt_issuer
  }

  # Etiquetas de los recursos (buena práctica de infra: costo, filtrado y
  # trazabilidad). Scaleway usa tags planos key=value.
  function_tags = [
    "project=perfil-api",
    "stage=${var.stage}",
    "managed-by=terraform",
  ]
}

# Un namespace: `perfil-api-production` (D16 — staging no existe).
resource "scaleway_function_namespace" "perfil" {
  name   = "perfil-api-${var.stage}"
  region = var.scw_region

  environment_variables = local.function_environment
  tags                  = local.function_tags
}

# La función `api` con sus credenciales DECLARADAS (revisión de D16 — el
# hallazgo del primer deploy: el PUT del provider borraba los secrets
# adjuntados a mano en la consola; declarados, el PUT los lleva siempre).
# Los valores viajan cifrados como secrets de GitHub y el state guarda los
# hashes — nunca el repo ni los logs.
resource "scaleway_function" "api" {
  namespace_id = scaleway_function_namespace.perfil.id
  name         = "api"
  region       = var.scw_region

  runtime      = "rust196"
  handler      = "handle"
  privacy      = "public"
  http_option  = "enabled"
  sandbox      = "v2"
  min_scale    = 0
  max_scale    = 1
  memory_limit = 128
  timeout      = 30

  zip_file = "${path.module}/function.zip"
  zip_hash = filesha256("${path.module}/function.zip")

  # Credenciales de la función (sensitive: el plan y los logs las enmascaran).
  secret_environment_variables = {
    DATABASE_URL             = var.database_url
    SUPABASE_JWKS_URL        = var.supabase_jwks_url
    SERVICE_ROLE_KEY         = var.service_role_key
    SUPABASE_PUBLISHABLE_KEY = var.supabase_publishable_key
  }

  tags = local.function_tags
}