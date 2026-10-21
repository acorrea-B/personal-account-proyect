# Contrato HTTP: `/v1/auth/*`

**Date**: 2026-10-21 · Errores: [errors.md](./errors.md). Rutas públicas (FR-011):
register, resend-verification, login, refresh. Ruta protegida: revoke.
Todos aceptan/responde `application/json; charset=utf-8`.

## POST /v1/auth/register — registro (FR-001..003, US1)

**Request** `201` en éxito:
```json
{ "email": "  juan@Example.com ", "password": "contraseña1" }
```
Normalización previa: `trim` + `lowercase` del email (FR-002). La contraseña se envía
tal cual (sin trim).

**Response 201** — datos del usuario, **nunca credenciales** (FR-003):
```json
{
  "user": {
    "id": "3f2504e0-4f89-41d3-9a0c-0305e82c3301",
    "email": "juan@example.com",
    "verified": false,
    "created_at": "2026-10-21T12:00:00Z"
  }
}
```
Efecto lateral: email de verificación (R5). `email` en respuesta ya normalizado.

**Errores**: `email-malformed` (422) · `weak-password` (422, con requirements) ·
`email-exists` (409, case-insensitive) · `rate-limited` (429) ·
`dependency-unavailable` (503 — cuenta creada, email pendiente: el reenvío cubre).

## POST /v1/auth/resend-verification — reenvío (FR-005)

**Request**:
```json
{ "email": "juan@example.com" }
```
**Response 200** — genérica SIEMPRE (FR-006):
```json
{ "message": "Si el email corresponde a una cuenta pendiente, se envió un enlace." }
```

**Errores**: `email-malformed` (422) · `rate-limited` (429 — límite estricto,
R2: 3/10min) · `dependency-unavailable` (503).

## POST /v1/auth/login — inicio de sesión (FR-006, US2)

**Request**:
```json
{ "email": "juan@example.com", "password": "contraseña1" }
```
**Response 200**:
```json
{
  "access_token": "<jwt supabase hs256>",
  "refresh_token": "<token opaco supabase>",
  "expires_in": 3600,
  "user": { "id": "...", "email": "juan@example.com", "verified": true,
            "created_at": "..." }
}
```

_Nota: `expires_in` es el TTL configurado en el proyecto Supabase (el ejemplo muestra
su default, 3600) — tratarlo como valor dinámico de cada respuesta, no hardcodearlo_.

**Errores**: `invalid-credentials` (401 — **idéntico y temporalmente comparable** para
password incorrecta / email inexistente, SC-004) · `email-not-verified` (403, con
`can_resend: true`) · `rate-limited` (429 — 5/login-fallido/min, R2) ·
`bad-request` (400, body malformado).

## POST /v1/auth/refresh — renovación (FR-007/008)

**Request** (público; la credencial de renovación ES la autenticación):
```json
{ "refresh_token": "<token>" }
```
**Response 200**: misma forma que login (par nuevo).

**Errores**: `invalid-refresh-token` (401 — expirado/no existe) ·
`refresh-token-reused` (401 — la sesión completa se invalida) · `bad-request` (400).

## POST /v1/auth/revoke — revocación (FR-009)

**Protegido**: requiere `Authorization: Bearer <access_token>`.

**Request**: body vacío (o `{}`).

**Response 204** — sin body.

**Errores**: `unauthorized`/`token-expired`/`token-revoked` (401) ·
`dependency-unavailable` (503).

## Nota sobre logout

Este contrato no define logout adicional: `revoke` cumple ese rol en la beta
(una sola ruta, DRY). Clientes que solo descarten credenciales pueden hacerlo
sin servidor.