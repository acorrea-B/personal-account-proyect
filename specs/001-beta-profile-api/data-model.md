# Data Model: Beta API de Perfil Profesional

**Date**: 2026-10-21 · **Spec**: [spec.md](./spec.md) · **Plan**: [plan.md](./plan.md)

Fuente del esquema: `docs/plan.md` §6. Este documento agrega el mapeo spec ↔ entidades,
reglas de validación y transiciones de estado.

## Entidades

### 1. auth.users *(gestionada por Supabase, no en migraciones propias)*

| Aspecto | Detalle |
|---------|---------|
| Rol | Única fuente de identidad (D3/Principle V). La API nunca la modifica con SQL directo — solo vía Supabase Auth admin |
| Campos usados | `id` (uuid, PK global), `email`, `email_confirmed_at`, `created_at` |
| Relaciones | 1:1 → `profiles`, `cv_texts`; 1:n → `cv_versions`; todas con FK `on delete cascade` |

### 2. profiles (espejo de identidad)

| Campo | Tipo | Regla |
|-------|------|-------|
| id | uuid PK | FK a auth.users, cascade |
| email | text not null | espejo normalizado (lowercase+trim, D4/ADR 0006); índice único `lower(email)` |
| full_name | text not null | 1..200 chars tras trim; required en `PUT /v1/profile`; inicial: placeholder "Usuario" (Clarifications Q1) |
| headline | text | ≤ 120 chars |
| bio | text | ≤ 2000 chars |
| location | text | ≤ 120 chars |
| links | jsonb | `[{"type":≤40chars, "url":http(s) válido ≤2048}]`; máx 10 entradas |
| skills | jsonb | `["texto ≤40 chars"]`; máx 30 entradas, sin duplicados |
| processing_consent | boolean null | reservado era LLM (D15); sin lógica v1 |
| created_at / updated_at | timestamptz | `updated_at` por trigger en UPDATE |

Validación: estructural en Rust (R3) — la DB solo valida tipo jsonb. Creación en el
momento de confirmación de email (FR-004) vía **trigger de DB sobre `auth.users`
(AFTER UPDATE OF email_confirmed_at WHEN NEW.email_confirmed_at IS NOT NULL)` →
insert espejo con email normalizado y `full_name="Usuario"` (Clarifications Q1) —
mecanismo elegido en research R11 (testeable en Supabase local, sin webhooks)

### 3. cv_texts (versión vigente)

| Campo | Tipo | Regla |
|-------|------|-------|
| id | uuid PK | |
| user_id | uuid unique | FK a auth.users, cascade → 1 fila máx por usuario |
| content | text | not null, CHECK `length ≤ 51200` bytes (50KB, FR-015); UTF-8 validado en handler |
| created_at / updated_at | timestamptz | |

### 4. cv_versions (historial, ADR 0013)

| Campo | Tipo | Regla |
|-------|------|-------|
| id | uuid PK | |
| user_id | uuid | FK a auth.users, cascade |
| content | text | not null, CHECK ≤ 51200 |
| origin | text | not null default 'user', CHECK in ('user','ai') — 'ai' solo se escribe post-beta (D15) |
| archived_at | timestamptz | default now() |

Índice: `(user_id, archived_at desc)`. Retención: máx 10 filas por usuario tras cada
escritura (purga en la misma transacción, FR-016/017 — nunca job aparte).

### 5. Sesión *(estado en Supabase Auth, no tabla propia)*

Vida: credencial de uso (JWT HS256, `sub`=user_id, `exp`, `iss`, `aud`) + credencial de
renovación. Transiciones manejadas por Supabase (tabla `auth.sessions`, rotación one-use
con detección de reuso — FR-008). Nuestra capa: validación (middleware auth.rs) y
revocación vía admin API (FR-009).

## Transiciones de estado

**Cuenta** (FR-003/004):
```
[register] → UNVERIFIED (perfil NO creado)
[confirm email] → VERIFIED → perfil creado (espejo completo)
```

**Sesión** (FR-008/009/010):
```
[login] → ACTIVE(access, refresh)
[refresh] → ACTIVE(access', refresh') — anterior refresh inválido
[reuse refresh] → REVOKED(session completa)
[revoke] → REVOKED — access ya no pasa middleware
```

**CV** (FR-015/016/017):
```
[POST sin fila] → cv_texts nueva + sin archivado (primera versión)
[POST con fila] → tx: {archiva anterior en cv_versions(user, origin='user')} 
                  {upsert cv_texts} {purga >10 versiones}
GET versions → lista paginada implícita (≤10)
```

**Concurrencia de escritura (Clarifications Q2)**: `PUT /profile` y `POST /cv/text` son optimistas — el handler compara el `updated_at` del cliente (`If-Unmodified-Since`) con el vigente; en desacuerdo responde 412 `stale-version` sin modificar nada. La verificación ocurre DENTRO de la transacción del CV (la tx es el árbitro final).

**Todo-or-nothing**: la tx del CV es el único punto multi-tabla de escritura —
todo-or-nothing (edge case spec: DB sin respuesta → 503, nada a medias, SC edge).

## Mapeo spec ↔ entidades

| Requisito | Entidad/operación | Test asociado |
|-----------|-------------------|---------------|
| FR-002 (unicidad case-insens.) | índice `lower(email)` + normalización pre-insert | tests/integration/register.rs |
| FR-004 (perfil al verificar) | trigger/hook de confirmación | tests/integration/auth_flow.rs |
| FR-016/017 (transacción CV) | tx en db/cv_texts.rs | tests/integration/cv_flow.rs (reemplazos encadenados, 11+ reemplazos) |
| FR-019 (anti BOLA) | filtro user_id en cada query de db/ | tests/security/bola.rs (SC-003, por endpoint) |
| SC-005 (10 versiones tras 40) | purga transaccional | tests/integration/cv_versions_retention.rs |