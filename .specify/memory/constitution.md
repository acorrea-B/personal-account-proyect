<!--
## Sync Impact Report
- Version change: 1.1.0 → 1.2.0 (MINOR: gate de CI con cobertura mínima 90% + ratchet, D17/ADR 0016)
- Modified: sección "Flujo de Trabajo" (CI/CD)
- Principles derived from: docs/plan.md §10 (Principios de Diseño), §11 (OWASP),
  §12 (Procesos), y decisiones D1-D15 (docs/adr/0001-0014)
- Added: 7 core principles; Security & Compliance section; Development Workflow section; Governance
- Removed: none (scaffold placeholders replaced)
- Deferred TODOs: promoción staging→producción (plan §14.5) — decidir en spec/plan
-->

# Perfil Profesional Serverless Constitution

## Core Principles

### I. DRY — Suficiente, no dogmático
Todo comportamiento compartido entre endpoints (validación JWT, rate limiting, logging,
mapeo de errores) vive en middleware tower o módulos compartidos; cada pieza de lógica
tiene y necesita un hogar autoritativo. La duplicación accidental se refiere a una falta
de abstracción, corregida en el PR que la introdujo — la lógica repetida entre dominios
debe extraerse antes del merge.

### II. KISS / YAGNI — Beta primero
Solo se construye lo del alcance beta v1; nada de post-beta/v2 (scopes, Storage, social
login, MFA, `DELETE /account`, `cv:process`, LLM runtime) se implementa — solo se deja
documentado su punto de extensión (plan §15, ADRs). Añadir abstracción "por si acaso"
procede en el PR que la necesite, no antes.

### III. Fail Fast
La validación ocurre en el borde (extractors + validadores) y los errores son tipados
(`thiserror`) con mapeo único en `errors.rs` (RFC 7807). `unwrap`/`expect` están
prohibidos en código de producción; los fallos de dependencias (DB, Supabase) se
degradan a 503 con retry acotado, nunca se tragan en silencio.

### IV. Seguridad por defecto (NON-NEGOTIABLE)
Deny-by-default: toda ruta es protegida salvo lista explícita de públicas en `router.rs`;
RLS deny-all bajo la lógica Rust (D3); toda query filtra por `user_id` del JWT validado.
Los checks de OWASP API Top 10 (plan §11) son criterios de aceptación — los tests de
acceso cruzado BOLA por endpoint son obligatorios y no negociables.

### V. Single Source of Truth
`auth.users` (Supabase) es la única fuente de identidad; `profiles` es espejo; `cv_texts`
pertenece siempre al `user_id` del token. Las convenciones se definen una vez y se
aplican sin excepciones: un formato de error (RFC 7807, D9), una tabla canónica de
status codes (plan §13.3), un estilo (rustfmt). Dos fuentes de verdad para lo propio
es un defecto que debe corregirse en el PR donde aparezca.

### VI. Convención sobre Configuración
Añadir un endpoint significa seguir el patrón existente: ruta en `router.rs`, handler en
`domain/`, DTO explícito, entry en repositorio, tests según §12.3 — sin decisiones
nuevas de estilo por archivo. La revisión es predecible porque el código es predecible.

### VII. Granularidad de Revisión
Máx 6 archivos por rama/PR — si una feature excede, se sub-divide en ramas apiladas con
pausa obligatoria para revisión manual iterativa (D10). Máx 2 structs/clases por
archivo, salvo centralizaciones de utils o enums. Los PRs deben ser revisables y el
feedback incorporado rama a rama.

## Seguridad y Cumplimiento

- Autenticación vía JWT de Supabase, validación: firma → exp → iss/aud → usuario existe,
  middleware tower compartido (D2, plan §3).
- Mensajería genérica anti-enumeración en auth pre-login; rate limiting de login y
  resend-verification; `service_role` key confinada en Scaleway Secret Manager.
- Datos personales: borrado de cuenta hard delete con cascade (D12, post-beta para el
  endpoint); retención de backups 30 días documentada; sin PII ni tokens en logs (§13.4).
- Columnas de preparación era LLM (`cv_versions.origin`, `profiles.processing_consent`,
  D15) se crean pero no se activan: `cv:process` exige `processing_consent = true`.

## Flujo de Trabajo

- **Git:** trunk-based (D10) — `main` deployable, ramas cortas `feature/*`, PR con CI y
  squash-merge, Conventional Commits. Reglas de granularidad de la Principle VII.
- **CI/CD (plan §12.2, D16, D17):** en PR: `cargo fmt --check`, `clippy -D warnings`,
  `cargo test` (unitarias + integración) con **cobertura mínima 90%** con ratchet
  (exclusiones solo con justificación en el PR), y `cargo audit` — todos obligatorios
  para merge. Merge a main despliega a staging;
  **producción SOLO por tag `vX.Y.Z`** con environment protegido y aprobación manual.
  Los secretos de la función viven exclusivamente en Scaleway Secret Manager — nunca
  pasan por GitHub ni por el repo; el pipeline solo los referencia por nombre.
- **Testing (plan §12.3):** unitarias (dominio/validación), integración (flujos auth
  completos contra Supabase local), matriz de seguridad obligatoria por endpoint
  (401/403/BOLA), todo edge case de plan §5 con al menos un test.
- **Decisiones (D11):** ADR en `docs/adr/` para toda decisión difícil de revertir o que
  afecte contrato público; numeración secuencial, inmutables (las correcciones son
  ADRs nuevos que marcan superseded).
- **Plan del proyecto:** `docs/plan.md` es el mapa; la constitución gobierna; los ADRs
  documentan el porqué.

## Governance

- La constitución supera a cualquier otra práctica o convenio del repo; en conflicto,
  gana la constitución — la discrepancia se reporta como issue de corrección.
- Enmiendas: ADR + cambio en este documento, con actualización de versión semántica
  (MAJOR: retrocompatible/incompatible con principios; MINOR: principio nuevo o
  ampliación material; PATCH: aclaraciones). La enmienda propone plan de migración
  para el código afectado.
- Todo PR y revisión del CI verifican cumplimiento: si un PR viola la constitución, se
  regresa a revisión con la violación señalada explícitamente.
- La granularidad de revisión (Principle VII) y el flujo de pausa aplican también a la
  generación de código por agentes: pausar para feedback del proyecto en cada rama.

**Version**: 1.2.0 | **Ratified**: 2026-10-21 | **Last Amended**: 2026-10-21