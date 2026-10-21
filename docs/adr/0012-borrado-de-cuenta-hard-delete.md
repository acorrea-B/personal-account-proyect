# ADR 0012: Borrado de cuenta — hard delete con cascade

**Estado:** Aceptada — 2026-10-21 (D12)

## Contexto
Había que definir la eliminación de cuentas (además: riesgo "Pérdida de datos CV" del plan y GDPR mínimo con usuarios EU).

## Opciones
- **A. Soft delete** (`deleted_at` nullable): recuperable, pero filtra en cada query (error fácil), la PII persiste (GDPR más complejo: requiere nullar datos) y el espacio nunca se libera.
- **B. Hard delete + cascade**: una operación contra `auth.users`; `profiles`, `cv_texts` y `cv_versions` se caen por `on delete cascade`. Datos realmente fuera de la DB.

## Decisión
Opción **B**. `delete from auth.users where id = $1` → cascade. GDPR mínimo: eliminación real.

## Consecuencias
- Los backups pg_dump de Supabase retienen datos borrados → **ventana de retención de backups: 30 días**, documentada en política de privacidad.
- Sin estados extra ni filtros en queries; el usuario que se arrepiente se registra de nuevo.
- **El endpoint `DELETE /v1/account` se difiere a post-beta.** El esquema ya lo contempla (FKs con `on delete cascade` definidas en la creación de tablas): cuando se active, es solo API + middleware de confirmación, sin ninguna migración. Durante la beta, las solicitudes de eliminación se atienden manualmente.