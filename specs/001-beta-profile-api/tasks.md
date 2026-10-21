---

description: "Task list template for feature implementation"
---

# Tasks: Beta API de Perfil Profesional

**Input**: Design documents from `/specs/001-beta-profile-api/`

**Prerequisites**: plan.md · spec.md · research.md · data-model.md · contracts/ · constitution

**Tests**: Incluidos explícitamente (D17: cobertura ≥90% con ratchet; matriz de seguridad obligatoria; edge cases del spec §5 con test asociado). Cada tarea de tests se escribe ANTES de la implementación que verifica (red → green).

## Formato: `[ID] [P?] [Story] Descripción con ruta`

- **[P]**: paralelizable (archivos distintos, sin dependencia de tareas incompletas)
- **[Story]**: user story dueña (US1..US4 según spec.md)
- Cada tarea lista las rutas exactas

## Reglas de ejecución (D10 — granularidad y pausas)

1. **Una tarea = una rama/PR**: rama `feature/<TXXX-slug>`, PR a `main`, squash-merge con Conventional Commit (`feat:`, `fix:`, `test:`, `chore:` — el tipo según la tarea).
2. **Máx 6 archivos por tarea** (ya garantizado: cada tarea lista sus archivos ≤6). Si durante la implementación un PR excede, dividir en ramas apiladas según ADR 0010.
3. **Pausa obligatoria al cerrar cada tarea**: PR abierto → revisión manual del proyecto + feedback → merge → continuar con la siguiente.
4. Orden de Phases respetado salvo tareas marcadas [P] dentro del mismo phase.

## Path Conventions

Proyecto único (crate Rust): `src/`, `migrations/`, `tests/` en la raíz del repo (ver plan.md).

---

## Phase 1: Setup (infraestructura compartida)

**Purpose**: inicializar el crate y la infraestructura del repo

- [X] T001 Scaffold del crate Rust: `Cargo.toml` (deps: axum, tokio, tower, tower-http, sqlx, serde, serde_json, validator, thiserror, tracing, tracing-subscriber, jsonwebtoken, reqwest, uuid, time), estructura de directorios del árbol de plan.md, `src/main.rs` bootstrap mínimo (config→pool→tracing→router→serve), `src/config.rs` (struct Config con env: DATABASE_URL, SUPABASE_URL, SUPABASE_JWT_SECRET, ENV, límites R2, pool, CORS — variables secretas solo por entorno según §12.2), `src/state.rs` (struct AppState: pool + cliente auth + rate limit)
- [ ] T002 [P] Imagen y pipelines: `Dockerfile` multi-stage (build release → imagen mínima), `.github/workflows/ci.yml` (gates PR: fmt --check, clippy -D warnings, cargo-llvm-cov --fail-under-lines 90, cargo audit — D16/D17), `.github/workflows/deploy.yml` (staging al merge a main, producción por tag vX.Y.Z con environments protegidos; migraciones como paso previo al deploy, R10; secrets SOLO referenciados por nombre desde Scaleway Secret Manager)
- [ ] T003 [P] Esquema inicial: `migrations/0001_schema.sql` con `profiles`, `cv_texts`, `cv_versions` y RLS deny-all exactamente como data-model.md (FKs cascade, CHECKs de longitud, índice `lower(email)`, índice `(user_id, archived_at desc)`, trigger `updated_at`), `migrations/0002_profiles_trigger.sql` (creación del espejo al confirmar email — FR-004)

---

## Phase 2: Foundational (bloqueantes — obligatorio antes de cualquier user story)

**⚠️ CRITICAL**: ningún user story inicia hasta completar esta fase.

- [ ] T004 Módulo de errores: `src/errors.rs` con structs `ApiError` y `ProblemBody` + catálogo completo de [contracts/errors.md](../specs/001-beta-profile-api/contracts/errors.md) (19 slugs ↔ status ↔ title ↔ extensiones; `can_resend` único en `email-not-verified`; `rate-limited` con `Retry-After`) — única fuente de mapeo, con tests unitarios inline del mapeo
- [ ] T005 Emisión de eventos: `src/events.rs` (struct Event + helper de emisión con envelope D18: ts/event/request_id/outcome/user_id/ip/meta; taxonomía cerrada de plan §13.5; `reason` = slugs del catálogo, R9) con tests unitarios del formato
- [ ] T006 Middleware de autenticación: `src/middleware/auth.rs` con trait `SignatureVerifier` (R1: HS256 con SUPABASE_JWT_SECRET tras el trait, ruta JWKS documentada), validación firma→exp→iss/aud, extractor tipo `AuthenticatedUser { user_id }` (deny-by-default), y `src/router.rs` actualizado con la distinción explícita pública/protegida (FR-011) + test de "ruta protegida sin token → 401 unauthorized"
- [ ] T007 Middleware de rate limiting: `src/middleware/ratelimit.rs` (layer in-memory por IP/user_id con expiración, límites env-configurables de R2: login-fallido 5/min, resend 3/10min, escritura CV 10/min, request 120/min; respuesta 429 canónica con Retry-After; emisión `security.rate_limited`) + tests unitarios con reloj simulado
- [ ] T008 Cliente Supabase Auth: `src/clients/supabase_auth.rs` (REST con reqwest + timeout 3s, retries acotados 2/backoff para 5xx — R7): signup, sign_in_with_password, token (refresh), logout/revoke admin; mapeo de respuestas de error de Supabase a slugs del catálogo (`invalid-credentials`, `email-exists`, `email-not-verified`, `invalid-refresh-token`, `refresh-token-reused`) + `src/main.rs` y `src/config.rs` actualizados para inyectarlo en AppState
- [ ] T009 Repositorios: `src/db/profiles.rs` (get/upsert espejo, query SIEMPRE filtrada por user_id — D3) y `src/db/cv_texts.rs` (get/upsert + transacción atómica archivar→upsert→purgar ≤10 — FR-016/017, SC-005; emisión `cv.versions_purged` con purged_count) + tests unitarios con sqlx test DB (mismo esquema T003)
- [ ] T010 Harness de integración: `tests/common/mod.rs` (arranque de la app sobre Supabase local de R8, fixture de creación de usuarios verificados/no verificados vía admin API, helper de peticiones con request-id) + `src/router.rs` completo con las 10 rutas del diseño colgadas de handlers stub `todo!()` que devuelven 503 — prueba de wiring: rutas responden 401/503 según corresponda ANTES de implementar dominios

**Checkpoint**: fondo listo — rutas existen, auth bloquea lo protegido, errores/eventos/repos tienen forma canónica. Los user stories pueden implementarse en orden.

---

## Phase 3: US1 — Registro con verificación de email (P1) 🎯 MVP

**Goal**: un visitante crea cuenta, verifica email y queda con perfil espejo creado.

**Independent Test**: register → confirmar → login bloqueado antes / permitido después — verificable sin perfil ni CV.

### Tests (primero, red)

- [ ] T011 [US1] Tests de registro: `tests/integration/register.rs` — 201 sin tokens (FR-003) · email normalizado en respuesta y DB (FR-002: espacios/mayúsculas) · `email-exists` 409 case-insensitive · `email-malformed` 422 · `weak-password` 422 con requirements (R4) · reenvío genérico 200 (FR-006) · login pre-verificación → 403 `email-not-verified` con `can_resend: true` · perfil creado tras verificar (FR-004)

### Implementación

- [ ] T012 [US1] DTOs y handlers de registro: `src/domain/auth/dto.rs` (structs RegisterRequest/RegisterResponse/ResendRequest con validación validator: email formato, password policy R4) y `src/domain/auth/handlers.rs` con `register` + `resend_verification` (llamadas al cliente T008, emisión `auth.registered`/`auth.verification_sent`, respuesta genérica SIEMPRE en resend — FR-005/006). IMPORTANTE: register NO crea el perfil espejo — se crea solo al confirmar email (FR-004, trigger T003 / test T013) + wiring en `src/router.rs`
- [ ] T013 [US1] Verificación de perfil espejo: `tests/integration/profile_mirror.rs` — confirmar email dispara creación de `profiles` (email normalizado + full_name placeholder obligatorio del trigger T003) y unicidad 1:1; edge case "email service caído": cuenta creada, sin sesión, reenvío disponible (spec §5)

**Checkpoint**: US1 funcional e independiente — MVP demostrable (quickstart §4-A parte 1).

---

## Phase 4: US2 — Login y sesiones renovables/revocables (P2)

**Goal**: usuario verificado inicia sesión, renueva sin re-login, revoca.

**Independent Test**: login → refresh → reuso rechazado → revoke → 401 — punta a punta sin perfil/CV.

### Tests (primero, red)

- [ ] T014 [US2] Tests de login/refresh/revoke: `tests/integration/session_flow.rs` — login 200 con par de tokens · credenciales incorrectas y email inexistente → mismo problem+json `invalid-credentials` y tiempos comparables (FR-006, SC-004) · login exitoso emite `auth.login_succeeded` / fallido `auth.login_failed` con reason (D18) · refresh 200 con par NUEVO y el anterior inválido (FR-008) · reuso → 401 `refresh-token-reused` y sesión completa invalidada · refresh expirado/malformado → `invalid-refresh-token`/`bad-request` · revoke 204 y acceso posterior → 401 (FR-009)

### Implementación

- [ ] T015 [US2] Handlers de sesión: extender `src/domain/auth/handlers.rs` con `login`, `refresh`, `revoke` (cliente T008; revocación vía admin API sobre user_id del token — FR-009; emisión `auth.session_refreshed`, `auth.refresh_rejected`, `auth.session_revoked`) y extender `src/domain/auth/dto.rs` (LoginRequest/TokenPair/RefreshRequest) + wiring `src/router.rs`
- [ ] T016 [US2] Rate limiting de auth en acción: `tests/integration/auth_ratelimit.rs` — 6º login fallido en la ventana → 429 `rate-limited` con Retry-After (R2) · 4º resend → 429 · verificación de emisión `security.rate_limited` con `key`/`route` correctas

**Checkpoint**: US1+US2 funcionan independientes y juntas (quickstart §4-A completo).

---

## Phase 5: US3 — Perfil profesional (P3)

**Goal**: usuario autenticado lee y actualiza su perfil con validación por campo.

**Independent Test**: login → GET perfil inicial → PUT válido → GET refleja cambio; PUT inválido → 422 por campo sin guardar.

### Tests (primero, red)

- [ ] T017 [US3] Tests de perfil: `tests/integration/profile_flow.rs` — GET devuelve espejo consistente (FR-012) · PUT completo guarda y refleja (semántica put completo del contrato) · PUT vacía full_name → 422 `profile-invalid-field` con `fields.full_name` · límites de data-model (headline/location 120, bio 2000, links ≤10 con type≤40 y url http(s) ≤2048, skills ≤30 sin duplicados ≤40) · campos omitidos quedan vacíos · escritura concurrente con `If-Unmodified-Since` desactualizado → 412 `stale-version` y nada guardado (Clarifications Q2) · respuestas sin campos internos (FR-013) · emisión `profile.updated` con campos modificados

### Implementación

- [ ] T018 [US3] Dominio de perfil: `src/domain/profile/dto.rs` (ProfileResponse/UpdateProfileRequest con reglas verbatim de data-model.md: "full_name 1..200 tras trim", "headline/location ≤120", "bio ≤2000", "links ≤10, type ≤40, url http(s) ≤2048", "skills ≤30, strings ≤40, sin duplicados") y `src/domain/profile/handlers.rs` con `get_profile`/`put_profile` (repositorio T009, filtro user_id, validación estructural R3, eventos D18) + wiring `src/router.rs`

**Checkpoint**: US1-3 funcionan (quickstart §4-B verde).

---

## Phase 6: US4 — CV como texto con historial (P4)

**Goal**: guardar/leer/reemplazar CV ≤50KB con archivado atómico y retención 10.

**Independent Test**: guardar → reemplazar → historial refleja versión previa con origin 'user' → 40 reemplazos dejan exactamente 10.

### Tests (primero, red)

- [ ] T019 [US4] Tests de CV: `tests/integration/cv_flow.rs` — primer POST 201 con `previous_version_archived: false` · reemplazo archiva versión previa en historial con origin 'user' (FR-016) · límite exacto 51200 bytes aceptado / 51201 → 413 `cv-too-large` con max_bytes (edge spec) · contenido no-UTF8 → 422 `cv-invalid-encoding` · reescritura con `If-Unmodified-Since` desactualizado → 412 `stale-version`, sin archivado · GET sin CV → 404 `not-found` · GET devuelve texto íntegro · `cv.versions_purged` emitido con purged_count
- [ ] T020 [US4] Test de retención: `tests/integration/cv_versions_retention.rs` — 40 reemplazos encadenados → GET /v1/cv/versions tiene exactamente 10, más recientes primero (SC-005) · GET /v1/cv/versions/{id} de versión vigente → 200 con content · id vencida → 404 sin revelar existencia (FR-019) · purga ocurre en la misma transacción que el upsert (verificación: DB no conserva >10 ni a medias tras forzar fallo de la tx)

### Implementación

- [ ] T021 [US4] Dominio de CV: `src/domain/cv/dto.rs` (SaveCvRequest/SaveCvResponse/CvResponse/VersionMeta/VersionResponse) y `src/domain/cv/handlers.rs` con `post_text` (validación UTF-8→tamaño fail fast, precondition `If-Unmodified-Since` Q2 dentro de la tx, transacción del repo T009, event emisiones) · `get_text` · `list_versions` · `get_version` (anti-BOLA: WHERE user_id en las cuatro) + wiring `src/router.rs` (POST → 201, ADR 0007)

**Checkpoint**: US1-4 funcionan — alcance beta completo demostrable (quickstart §4-C verde).

---

## Phase 7: Matriz de seguridad cross-cutting (obligatoria, SC-003)

**Goal**: demostrar anti-BOLA y deny-by-default en TODOS los endpoints documentados.

- [ ] T022 Matriz BOLA: `tests/security/bola.rs` — usuarios A y B verificados con datos: para CADA ruta protegida (profile GET/PUT, cv POST/GET/versions/versions-id), B atacando objetos de A por body/path → 404/403 sin revelar existencia; para rutas públicas atacadas sin credencial → 401; token de issuer incorrecto → 401; token malformado → 400/401 según contrato; usuario eliminado → 401 (edge spec §5)
- [ ] T023 Gates de cobertura y audit en verde: `cargo llvm-cov --fail-under-lines 90` pasa con el árbol completo (D17) · `cargo audit` limpio · exclusión de bootstrap (`main.rs`/config) documentada en el PR con justificación (regla D17) — evidencia en notes del PR de T023

**Checkpoint**: SC-003 demostrado; gates de la constitución en verde.

---

## Phase 8: Polish & Cross-Cutting

- [ ] T024 Documentación de API: `docs/openapi.yaml` — OpenAPI 3.1 generado manualmente desde contracts/ (schemas DTO, problemas como responses con ejemplos del catálogo, securitySchemes bearer) + enlace desde `README.md`
- [ ] T025 [P] Deployment docs: `docs/deployment.md` — cómo provisionar Scaleway (namespaces staging/producción D16), secrets en Scaleway Secret Manager con los 5 nombres de §12.2 + variables no secretas, creación de los GitHub Environments protegidos, rotación 90 días; checklist manual de smoke test post-deploy (del quickstart) · **runbook de baja de cuenta en beta (FR-023)**: flujo de solicitud → borrado de `auth.users` con cascade (D12) por el operador → confirmación al solicitante → evidencia de la baja registrada (logs admin de Supabase)
- [ ] T026 Validación final del quickstart: ejecutar `specs/001-beta-profile-api/quickstart.md` completo contra staging, registrar resultado/evidencia e **incluir medición de latencia p95 por endpoint del contrato con carga ligera — SC-002 (p95 < 200ms)** en la evidencia — cierre de la definición de "done" de la feature

---

## Dependencies

```text
T001 ──┬── T002 [P], T003 [P]
       └── T004 ── T005
      T004,T005 ── T006 ── T007 [dentro del phase, tras T006]
      T008 (tras T004: mapeo a slugs) ── T009 (tras T003: esquema, T008: cliente)
      T009,T006 ── T010 ── (checkpoint FASE 2)
T010 ── T011 ── T012 ── T013   (US1)
T012 ── T014 ── T015 ── T016   (US2, usa cliente/DTOs de auth)
T010 ── T017 ── T018            (US3, usa repo perfiles)
T018 ── T019 ── T020 ── T021   (US4, usa repo cv + patrón de validación de perfil)
T021 ── T022 ── T023            (seguridad total, cobertura global)
T023 ── T024, T025 [P] ── T026
```

Orden completo de ejecución (una rama por tarea): T001 → T002 → T003 → T004 → T005 → T006 → T007 → T008 → T009 → T010 → T011..T013 → T014..T016 → T017..T018 → T019..T021 → T022 → T023 → T024/T025 → T026.

## Parallel Execution Examples

- **Phase 1**: T002 (Imagen/pipelines) y T003 (migraciones) corren en paralelo al finalizar T001.
- **US2/US3**: una vez T010: el agente de US2 (T014-T016) y el de US3 (T017-T018) tocan archivos disjuntos — pueden desarrollarse en ramas paralelas (merge US1 primero).
- **US4** necesita el patrón de validación de US3 (T018) — no paralelizar con US3.
- **Polish**: T024 y T025 paralelizables tras T023.

## Implementation Strategy

1. **MVP primero**: US1 (T001-T013) → demo: registro + verificación + login bloqueado — valor beta inmediato.
2. **Incremento de valor**: US2 (sesiones) → US3 (perfil) → US4 (CV) — cada checkpoint es deployable.
3. **Seguridad como cierre de fase obligatorio** (T022-T023): la matriz BOLA y la cobertura ≥90% gatean el "done" del alcance, no un extra final.
4. **Validación en cada checkpoint**: quickstart.md parcial; T026 la completa con staging.

**Total: 26 tareas** · US1: 3 (T011-T013) · US2: 3 (T014-T016) · US3: 2 (T017-T018) · US4: 3 (T019-T021) · Setup: 3 · Foundational: 7 · Seguridad: 2 · Polish: 3 · MVP sugerido: T001-T013.