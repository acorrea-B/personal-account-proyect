# Contrato: Catálogo de errores (RFC 7807 — D9/ADR 0009)

**Date**: 2026-10-21 · Este catálogo es la única fuente de tipos de error de la API.
Implementación única en `src/errors.rs`; documentación OpenAPI genera desde aquí.

## Formato común

Cada respuesta de error: `Content-Type: application/problem+json`.

```json
{
  "type": "https://api.<dominio>/errors/<slug>",
  "title": "<title>",
  "status": <status>,
  "detail": "<message in English>",
  "instance": "<ruta solicitada>"
}
```

Extensiones permitidas donde la tabla lo indica (p. ej. `can_resend`). Sin stack traces
ni mensajes técnicos internos jamás (§13.4). 500 siempre usa el mismo detalle genérico.

**Idioma**: los valores que ve el cliente (`title`, `detail` y valores de extensión)
van en inglés; la documentación de este catálogo, en español. Los `title` de la tabla
son ya literales del contrato y no se traducen.

## Catálogo canónico

| Slug | Status | Título (title) | Uso | Extensiones |
|------|--------|----------------|-----|-------------|
| `bad-request` | 400 | Bad Request | body/estructura JSON malformada; credencial de sesión sintácticamente inválida al renovar | — |
| `unauthorized` | 401 | Unauthorized | falta cabecera Authorization o credencial inválida | — |
| `token-expired` | 401 | Token Expired | credencial de uso expirada (cliente renovable) | — |
| `token-revoked` | 401 | Token Revoked | sesión revocada (logout/reuso detectado) | — |
| `invalid-credentials` | 401 | Invalid Credentials | login: contraseña incorrecta o email inexistente → **idéntico en ambos casos** (FR-006) | — |
| `invalid-refresh-token` | 401 | Invalid Refresh Token | renovación: token inexistente/expirado | — |
| `refresh-token-reused` | 401 | Refresh Token Reused | rotación: reuso detectado → sesión completa invalidada (FR-008) | — |
| `email-not-verified` | 403 | Email Not Verified | login con cuenta sin confirmar (FR-003) | `can_resend: true` |
| `forbidden` | 403 | Forbidden | operación no permitida para la identidad del token | — |
| `not-found` | 404 | Not Found | ruta o recurso propio inexistente (p. ej. CV sin guardar) | — |
| `email-exists` | 409 | Email Already Registered | registro con email ya usado (case-insensitive, FR-002) | — |
| `stale-version` | 412 | Precondition Failed | escritura optimista: recurso cambió desde la última lectura del cliente (version = `updated_at`, header `If-Unmodified-Since`) — nada se archiva ni sobrescribe | `current_updated_at` |
| `cv-too-large` | 413 | CV Too Large | contenido > 51200 bytes (FR-015) | `max_bytes: 51200` |
| `weak-password` | 422 | Weak Password | registro: no cumple requisitos publicados (R4) | `requirements: ["length ≥ 8", "≥ 1 letter", "≥ 1 number"]` |
| `email-malformed` | 422 | Malformed Email | registro/reenvío: formato de email inválido | — |
| `profile-invalid-field` | 422 | Profile Validation Failed | PUT perfil: campo(s) inválidos | `fields: { "<campo>": "<motivo>" }` |
| `cv-invalid-encoding` | 422 | CV Invalid Encoding | texto no UTF-8 válido (FR-015) | — |
| `rate-limited` | 429 | Too Many Requests | límites R2 excedidos | `retry_after_seconds: <n>` + cabecera `Retry-After` |
| `internal` | 500 | Internal Server Error | error no previsto — detalle genérico fijo | — |
| `dependency-unavailable` | 503 | Service Unavailable | DB o Supabase sin respuesta (edge case del spec) | — |

## Reglas de correlación

1. `email-not-verified` es el ÚNICO error con `can_resend`.
2. `invalid-credentials` no admite variante: ni campos, ni textos, ni timing distinto (FR-006, SC-004).
3. Todo slug corresponde a un test en `tests/security/` o `tests/integration/` (trazabilidad contract ↔ test).
4. Catálogo **cerrado**: agregar slug = PR que añade entrada aquí + `errors.rs` + test simultáneamente.