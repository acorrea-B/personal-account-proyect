# Implementation Plan: Beta API de Perfil Profesional

**Branch**: `001-beta-profile-api` | **Date**: 2026-10-21 | **Spec**: [spec.md](./spec.md)

**Input**: Feature specification from `/specs/001-beta-profile-api/spec.md`

**Nota de consolidación**: la arquitectura de sistema ya está decidida y documentada en
`docs/plan.md` (decisiones D1-D17) y `docs/adr/0001-0016`. Este plan NO duplica esa
documentación: referencia por ADR y agrega solo lo que la implementación necesita y el
plan de sistema aún no define (contratos DTO por endpoint, herramienta de cobertura,
patrones de detalle).

## Summary

Beta API para gestión de perfil profesional: registro con verificación de email
obligatoria, sesiones renovables/revocables, CRUD de perfil y CV de texto con historial
(≤10 versiones). Abordaje técnico: única función `api` (contenedor Rust, servidor HTTP
axum) desplegada en Scaleway Serverless; identidad y email verification gestionadas por
Supabase Auth; datos en Supabase PostgreSQL con acceso service-key y validación JWT +
filtrado por `user_id` en Rust (D3); errores RFC 7807 (D9). Especificación fuente:
`spec.md` (user stories P1-P4, FR-001..023, SC-001..007).

## Technical Context

**Language/Version**: Rust (toolchain estable, edición 2021)

**Primary Dependencies**: axum + tower/tower-http (D8, ADR 0008) · tokio · sqlx (Postgres,
queries verificadas) · serde/serde_json · validator (inputs) · thiserror · tracing +
tracing-subscriber (logs JSON) · jsonwebtoken + reqwest (Supabase Auth admin) ·
cargo-llvm-cov (cobertura ≥90%, D17) · cargo-audit

**Storage**: Supabase PostgreSQL (esquema en [data-model.md](./data-model.md); migraciones
`sqlx migrate` en el repo)

**Testing**: `cargo test` (unitarias + integración) + `cargo-llvm-cov --fail-under-lines 90`
(D17/ADR 0016); Supabase local vía CLI para integración; matrices de seguridad obligatorias

**Target Platform**: Contenedor Linux en Scaleway Serverless Functions (staging +
producción); deploy CI/CD por tags (D16/ADR 0015)

**Project Type**: web-service — función monolítica `api` con router interno bajo `/v1` (D6)

**Performance Goals**: p95 < 200ms en operaciones auth/perfil/CV (SC-002); cold start
< 500ms (métrica §9)

**Constraints**: costo beta < €5/mes (SC-006) · CV ≤ 50KB · rate limits por IP/usuario ·
secrets exclusivamente en Scaleway Secret Manager (nunca por GitHub/repo, D16) · RLS
deny-all como red de seguridad (D3) · cobertura ≥90% con ratchet (D17)

**Scale/Scope**: beta ≤ 1k usuarios; 9 rutas + 2 de versiones; mensajes de usuario en
español, contrato de errores agnóstico del idioma

## Constitution Check

*GATE: Must pass before Phase 0 research. Re-check after Phase 1 design.*

| Principio | Evaluación |
|-----------|------------|
| I. DRY | ✅ Middleware tower compartido (auth, rate limit, tracing), un módulo `errors.rs` como única fuente de mapeo |
| II. KISS/YAGNI | ✅ scopes/Storage/social/MFA/cv:process fuera de la v1 con puntos de extensión documentados (plan §15); columnas D15 reservadas sin lógica |
| III. Fail Fast | ✅ Validación en el borde (extractors + validator); `thiserror`; prohibido unwrap/expect en producción |
| IV. Seguridad por defecto | ✅ deny-by-default en router (lista explícita de públicas); RLS deny-all; filtro user_id en toda query; matriz BOLA obligatoria (SC-003) |
| V. Single Source of Truth | ✅ `auth.users` única identidad; `profiles` espejo; convenciones únicas (RFC 7807, tabla canónica de status) |
| VI. Convención sobre Configuración | ✅ Estructura de crate §13.2 fija; nuevo endpoint = patrón existente |
| VII. Granularidad de Revisión | ✅ El desglose en ramas/PRs ≤6 archivos será responsabilidad de /speckit-tasks — este plan mantiene archivos pequeños (≤2 structs) para viabilizarlo |

**Re-check post-fase-1**: ✅ Los contratos (contracts/) usan un único formato problem+json
con catálogo centralizado de tipos de error — sin violaciones. Sin violaciones que
justificar en Complexity Tracking.

## Project Structure

### Documentation (this feature)

```text
specs/001-beta-profile-api/
├── plan.md              # This file (/speckit-plan command output)
├── research.md          # Phase 0 output (/speckit-plan command)
├── data-model.md        # Phase 1 output (/speckit-plan command)
├── quickstart.md        # Phase 1 output (/speckit-plan command)
├── contracts/           # Phase 1 output (/speckit-plan command)
│   ├── errors.md        # Catálogo canónico de problemas (slug ↔ status ↔ title)
│   ├── http-auth.md     # Contrato /v1/auth/*
│   ├── http-profile.md  # Contrato /v1/profile
│   └── http-cv.md       # Contrato /v1/cv/*
└── tasks.md             # Phase 2 output (/speckit-tasks command - NOT created by /speckit-plan)
```

### Source Code (repository root)

```text
src/
├── main.rs                  # bootstrap: config, pool, tracing, servidor [excl. cobertura]
├── config.rs                # Config (env): Struct
├── state.rs                 # AppState (pool + clientes): Struct
├── router.rs                # mapping de rutas + lista explícita de públicas: fn
├── errors.rs                # ApiError (RFC 7807) + mapeos [centralización: structs ApiError, ProblemBody + catálogo]
├── events.rs                # emisión de eventos trazables (envelope D18) [centralización]
├── middleware/
│   ├── auth.rs              # gate JWT (firma→exp→iss/aud→usuario existe→user_id): Struct AuthLayer + extractor
│   └── ratelimit.rs         # rate limiting por IP/usuario (in-memory beta): Struct RateLimitLayer
├── domain/
│   ├── auth/
│   │   ├── handlers.rs      # register, resend, login, refresh, revoke (1 handler/fn)
│   │   └── dto.rs           # request/response DTOs de auth [centralización DTOs]
│   ├── profile/
│   │   ├── handlers.rs      # get_profile, put_profile
│   │   └── dto.rs
│   └── cv/
│       ├── handlers.rs      # post_text, get_text, list_versions, get_version
│       └── dto.rs
├── db/
│   ├── profiles.rs          # repositorio perfiles
│   └── cv_texts.rs          # repositorio CV + versiones (tx atómica FR-016/017)
└── clients/
    └── supabase_auth.rs     # cliente Supabase Auth admin (con timeouts/backoff)
migrations/                  # sqlx migrate (esquema data-model.md)
tests/
├── integration/             # flujos end-to-end (Supabase local): auth, perfil, cv
└── security/                # matriz obligatoria: 401/403/BOLA por endpoint (SC-003)
.github/workflows/
├── ci.yml                   # gates PR: fmt, clippy, test+cobertura 90, audit
└── deploy.yml               # staging (merge main) + producción (tag vX.Y.Z)
Dockerfile                   # imagen del contenedor (multi-stage, release)
```

**Structure Decision**: proyecto único (crate Rust) — coincide con la función monolítica
D6/ADR 0004. La separación por dominio (`domain/{auth,profile,cv}`) y `db/` prepara la
extracción futura a N funciones si crece (§15), sin coste en v1. Cada archivo respeta
≤2 structs (D10); las centralizaciones permitidas están marcadas en el árbol
(`errors.rs`, `dto.rs`, `config.rs`).

## Complexity Tracking

> Sin violaciones de constitución que justificar — la tabla no aplica.