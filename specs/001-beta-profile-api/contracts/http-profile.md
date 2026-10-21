# Contrato HTTP: `/v1/profile`

**Date**: 2026-10-21 · Errores: [errors.md](./errors.md). Ambas rutas PROTEGIDAS
(FR-011): `Authorization: Bearer <access_token>` obligatorio. Errors de autenticación
comunes a ambas: `unauthorized` · `token-expired` · `token-revoked` (401) ·
`rate-limited` (429).

## GET /v1/profile — consultar (US3, FR-012/013)

**Response 200** — el perfil del `user_id` del token, y solo ese:
```json
{
  "profile": {
    "id": "3f2504e0-4f89-41d3-9a0c-0305e82c3301",
    "email": "juan@example.com",
    "full_name": "Juan Perez",
    "headline": "Desarrollador backend",
    "bio": "10 años en sistemas distribuidos...",
    "location": "Buenos Aires, AR",
    "links": [
      { "type": "linkedin", "url": "https://www.linkedin.com/in/juanperez" }
    ],
    "skills": ["rust", "aws", "postgres"],
    "created_at": "2026-10-21T12:05:00Z",
    "updated_at": "2026-10-21T12:05:00Z"
  }
}
```

Campos de respuesta EXCLUSIVOS del contrato público (FR-013): jamás columnas de
auth.users distintas de `id`/`email`, ni `processing_consent` (D15, reservado).

**Errores**: autenticación (401/429). No hay `not-found`: la cuenta verificada SIEMPRE
tiene perfil (FR-004). Si la data lo permitiera, se registra como bug (no contract).

## PUT /v1/profile — actualizar (FR-012/014)

**Request** — `full_name` required; el resto opcional:
```json
{
  "full_name": "Juan Perez",
  "headline": "Desarrollador backend",
  "bio": "10 años...",
  "location": "Buenos Aires, AR",
  "links": [
    { "type": "linkedin", "url": "https://www.linkedin.com/in/juanperez" },
    { "type": "github",   "url": "https://github.com/juanperez" }
  ],
  "skills": ["rust", "aws"]
}
```
**Precondition (Clarifications Q2, escritura optimista):** cabecera `If-Unmodified-Since: <updated_at>` del perfil; si difiere del vigente → **412 `stale-version`** con `current_updated_at`, nada se guarda.

Semántica: **put completo** — los campos omitidos o `null` quedan vacíos (valor por
defecto del perfil). La actualización es del propio perfil (filtro `user_id`).

**Reglas de validación (FR-014, data-model.md):**
- `full_name`: 1..200 tras trim (required)
- `headline`/`location`: ≤ 120
- `bio`: ≤ 2000
- `links`: lista ≤ 10; cada `type` ≤ 40 chars, `url` http(s) válida ≤ 2048
- `skills`: lista ≤ 30; strings ≤ 40 chars, sin duplicados

**Response 200**: el perfil completo en la forma de GET.

**Errores**: `stale-version` (412) · `profile-invalid-field` (422, `fields: {"<campo>": "<motivo>"}` —
aviso por campo) · `bad-request` (400) · autenticación (401/429).