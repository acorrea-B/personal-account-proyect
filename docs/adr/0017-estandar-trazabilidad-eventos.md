# ADR 0017: Estándar de trazabilidad de eventos

**Estado:** Aceptada — 2026-10-21 (D18)

## Contexto
La trazabilidad existente era ad-hoc (access logs + FR-022 de errores). Faltaba un
estándar: qué se registra de todo lo que sucede (registro, login, verificación,
sesiones, perfil, CV, bloqueos) y con qué esquema común.

## Opciones
- **A. Solo esquema en logs (beta)**: taxonomía de eventos + envelope común aplicada a los logs estructurados existentes. Cero tablas, cero queries; investigable con el explorador de logs.
- **B. Logs + tabla `audit_events` ya**: auditoría consultable desde la app y sobrevive a rotación; agrega tabla + repo + escritura en cada request + política de purga.

## Decisión
Opción **A** en beta:

- **Envelope común** (obligatorios: `ts`, `event`, `request_id`, `outcome`; según evento: `user_id`, `ip`, `meta`).
- **Taxonomía cerrada**: `auth.*`, `profile.*`, `cv.*`, `security.*` (tabla completa en plan §13.5). Se amplía solo por PR con entrada + emisor + test — igual disciplina que el catálogo de errores.
- **Vocabulario único de motivos**: los `reason` de `meta` son siempre slugs del catálogo canónico de errores (`errors.md`) — nada nuevo que aprender.
- **Log-only**: se emite vía tracing estructurado; no hay tabla en beta.

Tabla `audit_events` quedará diferida (§15): al agregarse post-beta, es **aditiva** — mismo envelope, solo un escritor nuevo; el histórico de logs estándar no se pierde porque el estándar ya define la forma del dato.

## Consecuencias
- Toda investigación post-incidente parte de preguntas grep-pables: `event="auth.login_failed" reason="invalid-credentials"`.
- Correlación completa por `request_id` entre evento de negocio, access log y error de API.
- Trade-off aceptado: rotación de logs limita el horizonte de auditoría hasta que exista `audit_events` (§15).
- Privacidad: `ip` solo en eventos de seguridad/rate-limit; sin tokens ni PII (§13.4).