# Contrato HTTP: `/v1/cv/*`

**Date**: 2026-10-21 · Errores: [errors.md](./errors.md). Las cuatro rutas PROTEGIDAS
(FR-011): `Authorization: Bearer <access_token>`. Errores comunes:
`unauthorized`/`token-expired`/`token-revoked` (401) · `rate-limited` (429 — escritura
10/min/user, R2).

## POST /v1/cv/text — guardar (US4, FR-015/016/017)

**Precondition (Clarifications Q2, escritura optimista):** cabecera `If-Unmodified-Since: <updated_at>` con el `updated_at` de la última lectura/escritura del CV. Si difiere del vigente → **412 `stale-version`** con `current_updated_at`; nada se archiva. Primera escritura (sin CV previo) no requiere la cabecera.

**Request**:
```json
{ "content": "Juan Perez\nDesarrollador backend\n\nEXPERIENCIA\n..." }
```
Validación (en orden — fail fast): UTF-8 válido → tamaño ≤ 51200 bytes (a 51200 exacto
se acepta, edge case del spec).

**Response 201**:
```json
{
  "cv": {
    "id": "c9bf9e57-1685-4c89-bafb-ff5af830be8a",
    "updated_at": "2026-10-21T13:00:00Z"
  },
  "previous_version_archived": true
}
```
`previous_version_archived: false` en el primer guardado.

**Efecto transaccional (todo-o-nada — ADR 0013, data-model):** archiva la versión
anterior en historial con `origin='user'` → upsert de la vigente → purga versiones
fuera de retención (últimas 10 conservadas, SC-005).

**Errores**: `stale-version` (412) · `cv-invalid-encoding` (422) · `cv-too-large` (413, con `max_bytes`) ·
`bad-request` (400) · `dependency-unavailable` (503 — nada queda a medias).

## GET /v1/cv/text — leer la vigente

**Response 200**:
```json
{
  "cv": {
    "id": "c9bf9e57-...",
    "content": "Juan Perez\n...",
    "created_at": "2026-10-21T12:30:00Z",
    "updated_at": "2026-10-21T13:00:00Z"
  }
}
```
**Errores**: `not-found` (404 — sin CV guardado, es el único 404 esperable).

## GET /v1/cv/versions — listar historial (FR-018)

**Response 200** — solo metadatos, máximo 10, más recientes primero:
```json
{
  "versions": [
    { "id": "e7a2... ", "origin": "user", "archived_at": "2026-10-20T18:00:00Z" },
    { "id": "b2d9... ", "origin": "user", "archived_at": "2026-10-19T18:00:00Z" }
  ]
}
```

## GET /v1/cv/versions/{id} — leer una versión (FR-018)

**Response 200**:
```json
{
  "version": {
    "id": "e7a2...",
    "content": "texto de la versión archivada",
    "origin": "user",
    "archived_at": "2026-10-20T18:00:00Z"
  }
}
```

**Errores**: `not-found` (404 — id inexistente **o perteneciente a otro usuario**:
anti BOLA, FR-019 mismo tratamiento que el propio).

## Notas de seguridad (matriz obligatoria, SC-003)

Todas las rutas: un usuario B nunca puede leer/escribir objetos de A por ningún
parámetro (body, path, query) — la única llave es el `user_id` del token. Los tests
en `tests/security/bola.rs` ejercitan cada ruta aquí documentada con usuario ajeno
autenticado (incluye versión ajena 404 — que no revela existencia).