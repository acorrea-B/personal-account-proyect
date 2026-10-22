# Plan de Proyecto: Perfil Profesional Serverless en Scaleway

Versión consolidada tras iteración de decisions — 2026-10-21.

## 1. Visión General

Sistema de autenticación y gestión de perfil profesional en Rust sobre Scaleway Serverless Functions (handler `handle`; el runtime Rust de la plataforma compila el crate desde un zip), orientado a beta abierta.

**Alcance Beta (v1):**
- Registro y login con email + password (Supabase Auth)
- Verificación de email obligatoria
- Gestión de perfil profesional (CRUD)
- CV en texto plano (sin procesamiento)
- Refresh y revocación de sesión (manejado por Supabase Auth)
- Despliegue como una sola función `api`

**Stack:**
- Backend: Rust (axum + tokio — D8) sobre Scaleway Serverless Functions — el router axum es el núcleo interno y el handler `handle` se lo entrega a cada petición
- Errores: RFC 7807 `application/problem+json` (D9)
- Auth: Supabase Auth (JWT, refresh tokens, rotación)
- DB: Supabase PostgreSQL
- CI/CD: GitHub Actions
- Frontend: por definir (API frontend-agnóstica)

## 2. Decisiones de Arquitectura (record de iteración)

| # | Decisión | Elección | Notas |
|---|----------|----------|-------|
| D1 | Verificación de email | **Obligatoria** | Registro → 201 sin tokens; login sin confirmar → 403 |
| D2 | Scopes OAuth2 | **Diferidos a v2** | Tokens con todos los permisos del usuario; activación futura vía Custom Access Token Hook, sin migración |
| D3 | Acceso a datos | **Service key + validación en Rust** | RLS desactivada (o deny-all por defecto); cada query filtra por `user_id` del JWT validado |
| D4 | Normalización de email | **Solo en tabla espejo** | Supabase Auth ya normaliza y garantiza unicidad en `auth.users`; punycode diferido a social login |
| D5 | Respuesta de CV | **201 Created** | Flujo síncrono, no hay cola de procesamiento |
| D6 | Granularidad | **1 función monolítica `api`** | Router interno (axum) invocado por el handler `handle`; un deployment, un cold start, código compartido |
| D7 | Storage CV | **Columna TEXT en PostgreSQL** | Tabla `cv_texts`, límite 50KB; Storage queda como camino aditivo para PDFs en v2 |
| D8 | Framework Rust | **axum + tokio** | Middleware tower composable (gate JWT compartido, DRY); detalle en ADR 0008 |
| D9 | Formato de errores | **RFC 7807** | `application/problem+json`; detalle en §13.3 y ADR 0009 |
| D10 | Flujo de branches | **Trunk-based + PR** | Con reglas de granularidad: ≤6 archivos por PR (si excede, ramas apiladas con pausa de revisión manual); ≤2 structs por archivo salvo centralizaciones — ADR 0010 |
| D11 | Registro de decisiones | **ADRs** | `docs/adr/NNNN-*.md`; D1-D7 espejados retroactivamente — ADR 0011 |
| D12 | Borrado de cuenta | **Hard delete + cascade** | GDPR mínimo; retención de backups 30 días documentada — ADR 0012 |
| D13 | Historial CV | **Tabla `cv_versions`**, retención últimas 10 | Purga transaccional junto al upsert — ADR 0013 |
| D14 | Campos del perfil | **Texto + `links`/`skills` como jsonb** | Validación estructural en Rust; tablas auxiliares diferidas a v2 |
| D15 | Preparación era LLM | **Reservar `cv_versions.origin` + `profiles.processing_consent`** | Columnas sin lógica: capturan procedencia y consentimiento desde el primer dato; quotas/jobs/pgvector diferidos — ADR 0014 |
| D16 | CI/CD producción | **Deploy por tags `vX.Y.Z` en GitHub Actions** | Staging auto por merge; producción solo por tag con environment protegido; secrets solo en Scaleway Secret Manager (nunca por GitHub) — ADR 0015 |
| D17 | Gate de calidad | **Tests unitarios + integración, cobertura ≥90%** | Requisito de merge a main; cobertura con ratchet; exclusiones justificadas — ADR 0016 |
| D18 | Trazabilidad | **Estándar de eventos: taxonomía cerrada + envelope común, log-only en beta** | Taxonomía `auth.*`/`profile.*`/`cv.*`/`security.*`, reasons = slugs del catálogo de errores; tabla `audit_events` diferida (aditiva) — ADR 0017 |

## 3. API (v1)

Función `api`, todas las rutas bajo `/v1`:

| Método | Ruta | Auth | Descripción |
|--------|------|------|-------------|
| POST | /auth/register | ✗ | Registro; 201 `{ user }` sin tokens |
| POST | /auth/login | ✗ | Login; 200 con tokens; sin confirmar → 403 |
| POST | /auth/refresh | ✗ (refresh token) | Rotación de tokens (Supabase) |
| POST | /auth/revoke | ✓ | Revocar sesión |
| POST | /auth/resend-verification | ✗ | Reenvío de email (rate-limited anti email bombing) |
| GET | /profile | ✓ | Obtener perfil |
| PUT | /profile | ✓ | Actualizar perfil |
| POST | /cv/text | ✓ | Subir texto CV; 201; límite 50KB; archiva versión anterior |
| GET | /cv/text | ✓ | Leer texto CV (versión actual) |
| GET | /cv/versions | ✓ | Listar versiones archivadas (≤10, meta sin contenido) |
| GET | /cv/versions/{id} | ✓ | Leer una versión archivada |

**Middleware de autenticación (JWT de Supabase):** firma → `exp` → `iss`/`aud` → usuario existe → extraer `user_id` → handler.

**Autenticación previa al login (login/refresh/register):** mensajería genérica que no revele si el email existe.

**Formato de errores:** RFC 7807 (`application/problem+json`) en todos los endpoints, con extensiones propias — ver §13.3.

## 4. Flujos Principales

### 4.1 Registro
```
POST /auth/register
→ validar formato email
→ normalizar (lowercase, trim)
→ Supabase signup (hashea password)
→ email de verificación
→ 201 { user }  (sin tokens)
```

### 4.2 Login
```
POST /auth/login
→ normalizar email
→ Supabase valida credenciales
→ sin confirmar → 403 { error: "email_not_verified", can_resend: true }
→ confirmado → 200 { access_token, refresh_token }
```

### 4.3 Refresh
```
POST /auth/refresh
→ Supabase valida refresh token (rotación one-use con detección de reuso)
→ 200 { nuevo par de tokens }
```

### 4.4 Petición autenticada
```
Authorization: Bearer <jwt>
→ validar firma (Supabase JWT secret)
→ validar exp / iss / aud
→ usuario existe
→ extraer user_id → handler filtra por user_id
```

### 4.5 CV texto
```
POST /cv/text
→ validar ≤ 50KB, UTF-8 limpio
→ transacción: archivar versión anterior en cv_versions (si existía)
             → upsert cv_texts (1 fila por usuario)
             → purgar cv_versions fuera de retención (últimas 10)
→ 201 { cv_id, updated_at }
```

## 5. Edge Cases

**Registro:** email existente → 409 · malformado → 422 · password débil → 422 con requisitos · DB timeout → 503 + retry · email service caído → usuario creado sin sesión, reenvío disponible.

**Login:** 3-5 intentos fallidos → bloqueo temporal · sesión no confirmada → 403 + can_resend · credenciales incorrectas → mensaje genérico · refresh expirado → re-login · token malformado → 400.

**Peticiones autenticadas:** expirado → 401 (client usa refresh) · revocado (session borrada) → 401 · usuario eliminado → 401.

**CV:** > 50KB → 413 · texto no-UTF8 → 422 · sin fila → 404.

**Anti-inyección:** queries exclusivamente parametrizadas.

## 6. Esquema DB (detalle — migraciones vía `sqlx migrate`)

```sql
-- === Seguridad (D3) ===
-- Las funciones usan service_role; RLS: deny-all como red de seguridad
-- (ninguna conexión anónima debe poder leer). El aislamiento vive en Rust.
alter table profiles  enable row level security;
alter table cv_texts  enable row level security;
alter table cv_versions enable row level security; -- sin políticas = deny-all

-- === Perfil profesional (espejo de auth.users; normalización propia, D4) ===
create table profiles (
  id          uuid primary key references auth.users(id) on delete cascade,
  email       text not null,                    -- espejo normalizado (lowercase + trim)
  full_name   text not null,
  headline    text,                             -- título profesional corto
  bio         text,
  location    text,
  links       jsonb,                            -- [{"type":"linkedin","url":"..."}]
  skills      jsonb,                            -- ["rust","aws",...]
  processing_consent boolean,                   -- consent. para futura IA: null=nunca preguntado · false=rechazó · true=aceptó (D15)
  created_at  timestamptz not null default now(),
  updated_at  timestamptz not null default now()
);
create unique index profiles_email_unique on profiles (lower(email));

-- === CV: versión actual (1 fila por usuario) ===
create table cv_texts (
  id          uuid primary key default gen_random_uuid(),
  user_id     uuid not null unique references auth.users(id) on delete cascade,
  content     text not null check (length(content) <= 51200),  -- 50KB
  created_at  timestamptz not null default now(),
  updated_at  timestamptz not null default now()
);

-- === CV: historial archivado (D13) ===
create table cv_versions (
  id          uuid primary key default gen_random_uuid(),
  user_id     uuid not null references auth.users(id) on delete cascade,
  content     text not null check (length(content) <= 51200),
  origin      text not null default 'user' check (origin in ('user', 'ai')),  -- procedencia del texto (D15)
  archived_at timestamptz not null default now()
);
create index cv_versions_user_recent on cv_versions (user_id, archived_at desc);
```

**Reglas del esquema:**
- **Creación de espejo:** `profiles` se crea (trigger/sign-up hook) al confirmar el email, garantizando un perfil por cuenta.
- **PUT /cv/text:** en **una transacción** — archiva el contenido anterior en `cv_versions` (si existía) y hace upsert de `cv_texts`.
- **Retención de versiones:** se conservan las **últimas 10** por usuario; la purga de las más viejas ocurre en la misma transacción (ADR 0013).
- **Borrado de cuenta (D12):** `delete from auth.users where id = $1` → `on delete cascade` elimina `profiles`, `cv_texts` y `cv_versions` — eliminación real de datos (GDPR mínimo). Los backups pg_dump de Supabase retienen datos borrados: **ventana de retención de backups = 30 días**, documentada en la política de privacidad.
- `links`/`skills` (jsonb) se validan estructuralmente en Rust al escribir (la DB solo valida tipo jsonb).

## 7. Seguridad

- JWT de Supabase: firma verificada con JWT secret (o JWKS), iss/aud/exp/iat/sub validados
- `service_role` key exclusivamente en secrets de Scaleway, nunca fuera de la función
- Rate limiting por IP y por usuario (3-5 intentos login; resend-verification estricto)
- Mensajes genéricos en credenciales inválidas (anti-enumeración) y anti timing attacks
- CORS restringido a dominios conocidos
- Secrets con Scaleway Secret Manager
- Queries parametrizadas siempre (SQLx)

## 8. Fases

**Fase 1 — Fundación:** workspace Rust · Scaleway CLI/namespaces · Supabase (Auth, DB) · estructura repo · CI/CD base.

**Fase 2 — Core:** integración Supabase Auth (register/login/refresh/revoke) · CRUD perfil · CV texto · middleware JWT · tests de integración.

**Fase 3 — Polish:** monitoreo/logging · docs API · deploy producción · beta · optimización cold starts.

## 9. Métricas de Éxito

- Costo mensual < €5 (beta)
- Cold start < 500ms · p95 < 200ms
- Disponibilidad > 99.9% · cero vulnerabilidades críticas
- Registro → perfil completo < 5 min

## 10. Principios de Diseño

- **DRY:** middleware tower compartido (JWT, rate limiting, logging), DTOs comunes, un solo módulo de errores (`errors.rs`) como única fuente de mapeo status ↔ respuesta. Lógica repetida entre dominios se extrae a módulos compartidos.
- **KISS / YAGNI:** nada de v2 (scopes, Storage, social login, cv:process) se construye en v1 — solo se deja el punto de extensión documentado.
- **Fail fast:** validación en el borde (extractors + validadores), errores tipados (`thiserror`), prohibido `unwrap`/`expect` en código de producción.
- **Seguridad por defecto:** deny-by-default — toda ruta es protegida salvo la lista explícita de públicas; RLS deny-all como red de seguridad bajo la lógica Rust (D3).
- **Single source of truth:** `auth.users` (Supabase) es la única fuente de identidad; `profiles` es espejo; `cv_texts` siempre pertenece a un `user_id` del token validado.
- **Convención sobre configuración:** un formato de error, una tabla de status codes, un estilo (rustfmt) — las convenciones se aplican sin excepciones para que la revisión sea predecible.

## 11. Seguridad — OWASP API Security Top 10 (2023)

| Riesgo | Amenaza | Mitigación en este proyecto |
|--------|---------|------------------------------|
| API1:2023 — BOLA | Acceder a objetos de otros usuarios | D3: toda query filtra por `user_id` del JWT validado; FKs `user_id` en `profiles`/`cv_texts`; **tests de acceso cruzado por endpoint** (obligatorios en Fase 2) |
| API2:2023 — Broken Authentication | Autenticación rota/suplantación | Tokens y rotación one-use de Supabase con detección de reuso; mensajería genérica anti-enumeración; rate limiting de login y resend |
| API3:2023 — Broken Property Level Auth | Exposición de propiedades sensibles | DTOs de respuesta explícitos por endpoint; nunca serializar columnas de `auth.users` ni metadata interna |
| API4:2023 — Unrestricted Resource Consumption | DoS por volumen/tamaño | Límite CV 50KB; `request-body-limit` en tower-http; rate limiting por IP/usuario; timeouts de request y query; pool acotado |
| API5:2023 — Broken Function Level Auth | Escalada a funciones admin | Gate JWT en middleware tower solo para rutas protegidas; matriz público/protegido como lista explícita en `router.rs`; tests por endpoint |
| API6:2023 — Sensitive Business Flows | Abuso de flujos de negocio | `resend-verification` con límites estrictos anti email-bombing; monitoreo de registro; captcha solo si se detecta automatización (futuro) |
| API7:2023 — SSRF | Requests falsificados a servicios internos | Regla dura: la API nunca hace fetch a URLs provistas por usuarios. Se re-evalúa con `cv:process` en v2 |
| API8:2023 — Security Misconfiguration | Configuración insegura | CORS restringido a dominios conocidos; secrets en Scaleway Secret Manager; headers de seguridad; sin stack traces al cliente |
| API9:2023 — Improper Inventory Management | Endpoints viejos expuestos | Versionado `/v1` desde el día 1; docs OpenAPI generadas; entorno staging separado de producción |
| API10:2023 — Unsafe Consumption of APIs | Consumo inseguro de APIs externas | Único externo: Supabase — respuestas validadas, timeouts y retries acotados con backoff |

## 12. Procesos

### 12.1 Git y contribución (trunk-based — D10)
- `main` siempre deployable; despliega vía CI cuando está verde
- Trabajo en ramas cortas `feature/*`; PR hacia `main` con squash-merge; Conventional Commits (`feat:`, `fix:`, `docs:`, `chore:`, `refactor:`)
- **Regla de granularidad:** máx **6 archivos** por rama/PR. Si una feature excede, se sub-divide en ramas apiladas (ej: CRUD de 9 archivos → "CR" 5 archivos → PR → revisión manual + feedback → merge → "UD" 4 archivos → PR). **Pausa obligatoria en cada rama** para revisión iterativa
- **Regla de estructura:** máx **2 structs/clases por archivo**, salvo centralizaciones de utils o enums

### 12.2 CI/CD (GitHub Actions) — despliegue por tags (D16)

**Entornos:** staging y producción como namespaces/funciones separadas en Scaleway, cada uno con su propio set de secrets (nombres idénticos, valores distintos).

| Etapa | Disparador | Qué hace |
|-------|-----------|----------|
| **CI (gates)** | PR hacia `main` | `cargo fmt --check` · `cargo clippy -D warnings` · `cargo test` (unitarias + integración) con **cobertura mínima 90%** (D17) · `cargo audit` — todo obligatorio para merge |
| **Deploy staging** | merge a `main` | build release + zip de la función (el runtime Rust de Scaleway compila el crate) → deploy de la función `migrate` → invocarla (migraciones) → deploy de la función `api` → smoke test básico (health + un flujo auth) |
| **Deploy producción** | push de tag `vX.Y.Z` (semver, anotado) | build release + zip → deploy de la función `migrate` → invocarla (migraciones) → deploy de la función `api` → smoke test → release notes del tag como changelog |
| **Rollback** | manual | re-desplegar el tag anterior (los deployments de función son inmutables por tag; reversion = re-desplegar la versión previa). Ojo: si el tag anterior migró el esquema, el rollback de código no deshace la migración — las migraciones deben ser retrocompatibles |

**Reglas del pipeline:**
- Los workflows de deploy se ejecutan con credenciales Scaleway (`SCW_ACCESS_KEY`/`SCW_SECRET_KEY`) guardadas en **GitHub Environment secrets** (`staging` y `production` como environments protegidos; producción requiere aprobación manual del environment)
- **Los secrets de la función NUNCA ni pasan por GitHub ni por el repo:** se adjuntan **fuera del pipeline** (consola de Scaleway o Terraform con SOPS) como `secret_environment_variables` cifradas por la plataforma; el pipeline solo actualiza el código y las variables no secretas. Scaleway no expone referencias a Secret Manager en Functions (solo en Jobs), de ahí que la adjunción sea manual. El pipeline nunca rota secrets: la rotación es una operación en Scaleway + Supabase seguida de un re-deploy de la misma versión
- Las versiones de la función son trazables (`api:{git-sha}` en staging, `api:{v-tag}` en producción); el pipeline registra el commit y el tag desplegados, y producción y staging nunca comparten versión en vuelo sin que el tag lo documente
- **Migraciones (R10):** paso previo obligatorio en ambos entornos — el pipeline despliega la función `migrate` (mismo crate, handler propio), la invoca y **solo si responde 200** actualiza la función `api`. Si la migración falla, la API sigue con la versión anterior. Las migraciones se ejecutan contra la **conexión directa** de Supabase (`MIGRATIONS_DATABASE_URL`), nunca contra el pooler: `sqlx` usa un advisory lock de sesión que el pooler en modo transacción no soporta
- Deploy deshabilitado por defecto en forks y en PRs externos
- **Cobertura (D17):** el gate corre cobertura de líneas sobre unitarias + integración con umbral **≥90%** global (herramienta: `cargo-llvm-cov`, decisión fina del plan de implementación); cobertura con ratchet — si `main` sube, no puede bajar. Código generado por macros externas y el bootstrap (`main.rs`/config) se excluyen del numerador con justificación en el PR

**Secrets que la función requiere para funcionar (cifrados en Scaleway, por entorno):**

| Secret | Uso |
|--------|-----|
| `SERVICE_ROLE_KEY` | clave `service_role` de Supabase (acceso DB por D3) |
| `SUPABASE_JWT_SECRET` | verificación de firma de los JWT de Supabase Auth (o su JWKS URL si se elige rotación) |
| `SUPABASE_URL` | endpoint de Supabase (Auth admin, health) |
| `DATABASE_URL` | cadena de conexión al pooler de Supabase PostgreSQL (la API) |
| `MIGRATIONS_DATABASE_URL` | cadena de conexión **directa** a Supabase PostgreSQL (solo el handler `migrate`; el pooler no soporta el advisory lock de `sqlx`) |
| `SUPABASE_WEBHOOK_SECRET` | (opcional) verificación de callbacks/webhooks de Supabase si se usan |

Variables no secretas (config del deployment): `ENV`, límites de rate limiting, tamaño de pool, orígenes CORS. `PORT` solo aplica al servidor de desarrollo local (`cargo run`), no a la función.

**Rotación:** la `service_role` key y el JWT secret tienen política de rotación periódica (revisar cada 90 días); la rotación ocurre en Scaleway Secret Manager + Supabase, sin re-deploy (la función los lee en arranque o en refresco).

### 12.3 Estrategia de Testing
- **Unitarias:** dominio y validación (normalización, límites, DTOs)
- **Integración:** handlers contra DB de prueba (Supabase local vía CLI) — flujos auth completos: register → login → refresh → revoke
- **Matriz de seguridad (obligatoria):** por cada endpoint protegido, tests de 401 sin token, token expirado/revocado, y **acceso cruzado BOLA** (usuario A no ve objetos de B)
- **Edge cases:** cada caso de la §5 tiene al menos un test asociado

### 12.4 Registro de decisiones (ADR — D11)
- `docs/adr/NNNN-nombre.md` con formato: **Estado / Contexto / Opciones / Decisión / Consecuencias**
- Se crea ADR cuando una decisión es difícil de revertir o afecta contrato público
- Numeración secuencial; ADRs inmutables — las correcciones son nuevos ADRs que marcan los anteriores como superseded

## 13. Especificaciones Técnicas

### 13.1 Stack y dependencias
- Rust stable · **axum** · tower / tower-http (CORS, trace, timeout, request-body-limit) · **tokio**
- **sqlx** (PostgreSQL, migraciones, queries verificadas en tiempo de compilación)
- serde/serde_json · validator (inputs) · thiserror · tracing + tracing-subscriber (logs JSON)
- jsonwebtoken + JWKS de Supabase · reqwest (cliente Supabase Auth admin, con timeouts)
- Política: dependencias mínimas, `cargo audit` gate en CI

### 13.2 Estructura del crate (≤2 structs por archivo)
```
src/
  lib.rs                   — handler de la función `handle` (axum) + `migrate`; entrega cada petición al router
  main.rs                  — servidor de desarrollo local (mismo router); no se despliega
  config.rs                — Config (env)
  state.rs                 — AppState (pool + clientes)
  router.rs                — mapeo de rutas + lista explícita de públicas
  errors.rs                — ApiError (RFC 7807) y mapeos [centralización]
  middleware/
    auth.rs                — gate JWT
    ratelimit.rs           — rate limiting
  domain/
    auth/                  — handlers + DTOs de auth (1 por archivo)
    profile/               — handlers + DTOs de perfil
    cv/                    — handlers + DTOs de CV
  db/
    profiles.rs            — repositorio perfiles
    cv_texts.rs            — repositorio CV
  clients/
    supabase_auth.rs       — cliente Supabase Auth
```

### 13.3 Formato de errores (RFC 7807 — D9)
`Content-Type: application/problem+json` con `type`, `title`, `status`, `detail`, `instance`; extensiones propias permitidas (ej. `can_resend`, errores por campo). `type` = `https://api.<dominio>/errors/<slug>`.

Tabla canónica de status:

| Status | Uso |
|--------|-----|
| 400 | body/token malformado |
| 401 | no autenticado, token expirado/revocado |
| 403 | email no verificado, acceso a objeto de otro usuario |
| 404 | recurso inexistente |
| 409 | email ya registrado, conflicto |
| 413 | tamaño excedido (CV > 50KB) |
| 422 | validación de input |
| 429 | rate limit |
| 500 | error interno (sin detalle interno en respuesta) |
| 503 | dependencia caída (DB/Supabase) |

### 13.4 Logging y observabilidad
- tracing estructurado JSON; `request-id` por petición; `warn` para 4xx, `error` para 5xx
- Prohibido en logs: passwords, tokens, PII completa (email truncado/enmascarado)

### 13.5 Estándar de trazabilidad de eventos (D18 — log-only en beta)

**Envelope común** de todo evento de negocio (además del access log técnico):
```json
{ "ts": "ISO8601", "level": "info|warn", "event": "<taxonomía>",
  "request_id": "<uuid>", "user_id": "<uuid|null>", "ip": "<x|null>",
  "outcome": "accepted|rejected|blocked", "meta": { } }
```
- `ts`, `event`, `request_id`, `outcome` obligatorios; `user_id`/`ip` según el evento; `meta` mínimo necesario. Sin tokens, sin PII extra (§13.4). Emisión vía `tracing` como campo estructurado (`event=`), no texto libre.

**Taxonomía cerrada** (se amplía por PR, como el catálogo de errores):

| Evento | Emisor | meta típica |
|--------|--------|-------------|
| `auth.registered` | handler register | — |
| `auth.verification_sent` | resend (tras decisión genérica) | — |
| `auth.email_verified` | flujo de confirmación (perfil creado) | — |
| `auth.login_succeeded` / `auth.login_failed` | handler login | `reason`: reason del error canónico |
| `auth.session_refreshed` | handler refresh | — |
| `auth.refresh_rejected` | middleware/handler refresh | `reason`: `invalid-refresh-token` \| `refresh-token-reused` |
| `auth.session_revoked` | handler revoke | — |
| `profile.updated` | handler profile | `fields`: campos modificados |
| `cv.text_saved` | handler cv | `bytes`, `previous_version_archived` |
| `cv.versions_purged` | repositorio cv (tx) | `purged_count`, `kept=10` |
| `security.rate_limited` | middleware ratelimit | `route`, `key`: `ip`\|`user` |
| `security.auth_blocked` | middleware auth | `route`, `reason`: canon error |
| `security.bola_blocked` | — (reservado: se emite si un test o caso real intenta acceso cruzado forzado) | `route` |

**Reglas:**
1. Los `reason` de `meta` usan SIEMPRE slugs del catálogo de errores (contracts `errors.md`) — un solo vocabulario de motivos.
2. Taxonomía cerrada y versionada: agregar evento = PR con entrada aquí + emisor + test (igual que `errors.rs`).
3. Un evento por resultado significativo (no por capa); el `request_id` correlaciona con el access log.
4. `ip` solo en eventos de seguridad/rate limit; retención según la política de logs de la plataforma.
5. La tabla `audit_events` (auditoría consultable desde la app, sobrevive a rotación) queda diferida a post-beta — **aditiva**: el estándar/envelope ya existe, agregarla solo añade un escritor al mismo esquema (§15).

### 13.6 Límites operacionales
- Timeout de request 10s · timeout de query 5s · body máximo 64KB (50KB útiles de CV)
- Pool de conexiones ~5 por instancia · retries a Supabase acotados con backoff

## 14. Preguntas Abiertas (para el spec o fases siguientes)

1. Social login (Google/LinkedIn): recomendado diferir a v2
2. MFA: diferible; Supabase Auth lo soporta a posteriori sin migración
3. GDPR: durante la beta las eliminaciones de cuenta se atienden por solicitud manual; el endpoint `DELETE /account` (con su mecanismo de confirmación) es **post-beta** — el esquema ya lo contempla vía cascade (D12)
4. Frontend web/móvil: irrelevante para la API; decidir después

## 15. Diferido a post-beta / v2 (no se construye en v1, solo puntos de extensión documentados)

- Endpoints de scopes (D2 — activación vía Custom Access Token Hook)
- Supabase Storage para archivos (D7)
- Social login, MFA
- `DELETE /account` con mecanismo de confirmación (D12)
- `cv:process` y endpoints de resultados de CV (requiere `processing_consent = true`, D15)
- GDPR completo: export de datos de cuenta
- **Era LLM (D15):** contabilidad de uso/quota-credits por usuario (los costos por token exigen cap por cuenta) · cola de jobs asíncronos con patrón 202 + job status para `cv:process` · embeddings/semántica vía pgvector en el mismo Supabase Postgres (match profesional avanzado)
- **Auditoría persistente (D18):** tabla `audit_events` consultable desde la app (sobrevive a rotación de logs) — aditiva: escribe con el mismo envelope/taxonomía definidos en §13.5