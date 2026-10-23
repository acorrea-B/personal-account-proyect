# Outputs consumidos por el CI (smoke test) y por verificación manual.

output "function_endpoint" {
  description = "URL pública de la función `api` (target del smoke test)."
  value       = scaleway_function.api.domain_name
}

output "function_id" {
  description = "SRN de la función `api` (el smoke test consulta su estado de build)."
  value       = scaleway_function.api.id
}

output "namespace_name" {
  description = "Namespace de Scaleway del entorno."
  value       = scaleway_function_namespace.perfil.name
}