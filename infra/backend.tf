# Estado remoto de Terraform en Scaleway Object Storage (plan §12.2).
# El state pesa unos KB (namespace + función); el bucket debe existir antes
# del primer `terraform init` (no puede crearse con Terraform: bootstrapping):
#   scw object bucket create --region fr-par perfil-api-terraform-state
#
# Locking: el backend S3 de Terraform ≥1.10 bloquea con conditional writes si
# la API S3 del endpoint lo soporta; como garantía mínima, la `concurrency`
# del workflow de deploy serializa los apply por entorno.

terraform {
  backend "s3" {
    bucket = "perfil-api-terraform-state"
    key    = "perfil-api/terraform.tfstate"
    region = "fr-par"
    endpoints = {
      s3 = "https://s3.fr-par.scw.cloud"
    }

    # Las credenciales llegan por entorno: SCW_ACCESS_KEY -> AWS_ACCESS_KEY_ID
    # y SCW_SECRET_KEY -> AWS_SECRET_ACCESS_KEY (el backend S3 habla el
    # protocolo S3 genérico, no conoce el provider scaleway).
    skip_credentials_validation = true
    skip_requesting_account_id  = true
    skip_region_validation      = true
    skip_metadata_api_check     = true
    skip_s3_checksum            = true
    use_path_style              = true
  }
}