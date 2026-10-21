# ADR 0009: Formato de errores — RFC 7807 (problem+json)

**Estado:** Aceptada — 2026-10-21 (D9)

## Contexto
Quedaba abierta la forma en que la API comunica errores. Consecuencia directa sobre DX, documentación OpenAPI y consistencia entre endpoints.

## Opciones
- **A. RFC 7807** (`application/problem+json`): estándar IETF (`type`, `title`, `status`, `detail`, `instance`), entensible con campos propios, clientes estándar lo entienden.
- **B. Formato propio** `{ error: { code, message, details } }`: simple de escribir, pero cada cliente aprende un dialecto ad-hoc.

## Decisión
Opción **A**. `type` con URIs propios: `https://api.<dominio>/errors/<slug>`. Extensiones permisivas junto a los campos estándar (ej. `can_resend` en 403 de email no verificado). Tabla canónica de status codes en plan §13.3.

## Consecuencias
- Un único módulo `errors.rs` mapea `ApiError` → respuesta problem+json (DRY).
- OpenAPI 3.1 documenta los errores de forma estándar; OpenAPI valida la extensión de campos propia sin hacks.