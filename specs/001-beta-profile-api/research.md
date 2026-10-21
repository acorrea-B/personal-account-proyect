# Research: Beta API de Perfil Profesional

**Date**: 2026-10-21 · **Feature**: [spec.md](./spec.md) · **Plan**: [plan.md](./plan.md)

Fase 0 del /speckit-plan. Las decisiones macro ya están en `docs/adr/0001-0016`; aquí se
consolidan y se resuelven las decisiones **finas** restantes de implementación.

## Decisiones consolidadas (referencia a ADRs — no se re-evalúan)

| Tema | Decisión | ADR |
|------|----------|-----|
| Verificación email obligatoria | registro sin tokens, login → 403 + reenvío | 0001 |
| Scopes | diferidos; JWT con permisos completos | 0002 |
| Acceso a datos | service key + filtro user_id en Rust, RLS deny-all | 0003 |
| Granularidad | 1 función monolítica `api` | 0004 |
| CV en PG (columna TEXT) | no Storage en v1 | 0005 |
| Normalización email | solo en tabla espejo | 0006 |
| POST CV → 201 | síncrono | 0007 |
| Framework | axum + tokio | 0008 |
| Errores | RFC 7807 problem+json | 0009 |
| Git/PR | trunk-based, ≤6 archivos, ≤2 structs | 0010 |
| Decisión por ADR | docs/adr/ | 0011 |
| Borrado cuenta | hard delete cascade; endpoint post-beta | 0012 |
| Historial CV | cv_versions, retención 10, purga transaccional | 0013 |
| Era LLM | origin + processing_consent reservados | 0014 |
| CI/CD | deploy por tags; secrets solo en Scaleway SM | 0015 |
| Cobertura | ≥90% con ratchet, unitarias + integración | 0016 |

## Decisiones finas resueltas (nuevas en esta fase)

### R1. Verificación de firma de JWT: HS256 legacy secret (beta) con ruta JWKS documentada
**Decision**: Validar HS256 con `SUPABASE_JWT_SECRET` (secret legacy de Supabase) via
`jsonwebtoken`. El soporte JWKS (Supabase publicó migración a claves asimétricas) queda
como punto de extensión documentado: el extractor queda tras un trait
`SignatureVerifier` para swap sin tocar handlers.
**Rationale**: KISS en beta (una env var, sin caché ni fetch al arranque), y el secret
legacy sigue vigente en proyectos Supabase. El trait paga el costo futuro sin YAGNI
invertido hoy.
**Alternatives**: JWKS + cache kid (complejidad temprana, requiere reqwest al arranque);
ambos verificados en paralelo (redundancia innecesaria en beta).

### R2. Rate limiting: tower layer in-memory por instancia
**Decision**: Middleware in-memory (mapas por IP y por user_id, con expiración LRU) en
la instancia de la función. Límites configurables por env; llaves: IP para rutas
públicas, user_id para rutas protegidas + IP en auth.
**Rationale**: beta con una instancia; cero dependencias externas (KISS).
**Alternatives**: Redis/Upstash (YAGNI en beta, costo + pieza extra); nota documentada:
si Scaleway escala horizontalmente, los límites por instancia son el límite efectivo —
aceptable en beta con valores conservadores, swap futuro a un store compartido es
aditivo (mismo layer, otro backend).
**Límites iniciales** (env-configurables): login fallido 5/min/IP+user · resend 3/10min
· escritura CV 10/min/user · petición 120/min/IP.

### R3. Validación de inputs: crate `validator` + validaciones de dominio propias
**Decision**: `validator` para formato base (email, longitud, URL de links) y
funciones de dominio para reglas específicas (normalización email, límites CV,
estructura jsonb de links/skills).
**Rationale**: cubre el 80% mecánico con mensajes mapeables por campo (FR-013/014); las
reglas con semántica de negocio quedan explícitas y testeables en Rust puro.

### R4. Password policy en dos capas
**Decision**: pre-validación en Rust (longitud mínima 8, al menos una letra y un
número → 422 `/errors/weak-password` con requisitos) + la validación interna de
Supabase Auth como segunda capa (si llega a fallar ahí, se mapea al mismo tipo).
**Rationale**: FR-001 exige aviso específico de requisitos; depender solo del mensaje
de Supabase obligaría a parsear respuestas ajenas.

### R5. Envío de emails: templates nativos de Supabase Auth
**Decision**: verificación y reenvío usan el email provider integrado de Supabase
(smtp configurable si requerido), con los redirects configurados; el reenvío expone
respuesta **genérica** siempre (FR-006) — nunca revela existencia.
**Alternatives**: SMTP propio (operación extra, YAGNI); servicio transaccional externo
(pieza nueva pre-v2).

### R6. Cobertura: `cargo-llvm-cov`
**Decision**: herramienta de cobertura `cargo-llvm-cov` (line coverage), umbral
`--fail-under-lines 90`, corrida en el workflow de CI con caches de toolchain. Ratchet
implementado con job que guarda la métrica de main como artifact (D17).
**Rationale**: precisión LLVM nativa, soporta integration tests en el mismo cálculo,
mantenimiento activo.
**Alternatives**: `cargo-tarpaulin` (más lento, precisión variable con async/unsafe);
`cargo-coverage` abandonado.

### R7. Cliente de Supabase Auth: REST directo con reqwest
**Decision**: cliente propio pequeño en `clients/supabase_auth.rs` contra la Auth API
de Supabase (`/auth/v1/signup|token|logout|admin/...`) con reqwest + timeouts (3s) y
retries acotados (2, backoff) para 5xx/gateway.
**Rationale**: FR-022/III exigen control de errores y tracing; el SDK disponible
(falta crate maduro para admin ops) no cubre todo y añade peso.
**Alternatives**: SDKs no oficiales (mantenimiento incierto); PostgREST directo para
auth (no soporta signup/token flows).

### R8. Local dev: Supabase CLI + docker compose
**Decision**: desarrollo local con Supabase local (CLI) y la función corriendo con
`cargo watch`; integración CI usa el mismo Supabase local en el job.
**Rationale**: mismos flujos/emails en local; reproductible en CI (quickstart.md
documenta los comandos).

### R9. Observabilidad: tracing JSON + request-id middleware + estándar de trazabilidad
**Decision**: `tracing-subscriber` JSON en producción/pretty en dev; middleware propio
que genera request-id (UUID) y lo coloca en logs y en cabecera `x-request-id`
(incluida en problem+json `instance` si aplica). Niveles: warn 4xx, error 5xx, info
requests. Redacción: email truncado, nunca tokens. **Actualización (D18/ADR 0017)**:
todo evento de negocio se emite con el envelope estándar y la taxonomía cerrada
(`auth.*`/`profile.*`/`cv.*`/`security.*`) del plan §13.5 — los `reason` reutilizan
slugs del catálogo de errores; el helper de emisión vive en un módulo compartido
(aplicación del DRY, un solo punto que forma el envelope).
**Rationale**: FR-022 + correlation para debug de beta sin PII (§13.4) + investigación
post-incidente estandarizada. Tabla `audit_events` diferida (aditiva).

### R11. Mecanismo de creación del perfil espejo (remediación F4 del /speckit-analyze)
**Decision**: trigger de base de datos sobre `auth.users` —
`AFTER UPDATE OF email_confirmed_at ... WHEN (NEW.email_confirmed_at IS NOT NULL)` —
que inserta el `profiles` espejo con email normalizado y `full_name` placeholder
"Usuario" (Clarifications Q1). Un único trigger (T003), testeable en Supabase local y
en la suite (T013), sin webhooks ni código de aplicación en la ruta de verificación.
**Rationale**: garantiza el espejo sin acoplar verify↔API, opera aunque la
confirmación llegue de rutas no propias (ej. admin), y mantiene FR-019 por diseño.
**Alternatives**: webhook Auth Hook de Supabase (punto extra de red/secret, reintentos
a operar); creación desde handler (no cubre confirmaciones externas).
**Nota**: register NO crea espejo (FR-004) — corregida la redacción de T012 (F1).

## NEEDS CLARIFICATION

Ninguno — resueltas D1-D18 + R1-R11.
**Decision**: `sqlx migrate` con migraciones versionadas en `migrations/`; el runner
del contenedor aplica migraciones al arrancar un job dedicado del deploy (no del
servidor en cada request) — el paso `migrate` corre como paso previo en
deploy.yml después del build y antes de redeploy de la función.
**Rationale**: separa migración de serving (fail fast en deploy, no en primer request).

## NEEDS CLARIFICATION

Ninguno — resueltas D1-D17 + R1-R10.