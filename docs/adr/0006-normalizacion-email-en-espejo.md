# ADR 0006: Normalización de email solo en la tabla espejo

**Estado:** Aceptada — 2026-10-21 (D4)

## Contexto
La sección 7.2 del plan original asignaba a nuestra capa la normalización (lowercase, trim, punycode) y unicidad case-insensitive del email. Pero el registro pasa por Supabase Auth, que ya normaliza y garantiza unicidad en `auth.users` — nuestra capa no controla ese flujo.

## Opciones
- **A. Duplicar normalización en nuestros handlers de auth**: redundante y contradictorio con lo que hace Supabase aguas arriba.
- **B. Normalizar solo en la tabla espejo**: lower+trim al insertar el `profile`; índice único case-insensitive en esa tabla para búsquedas/dedup del lado de la app. Punycode (dominios IDN) diferido a social login.

## Decisión
Opción **B**. La unicidad de la identidad vive en `auth.users`; la normalización propia aplica únicamente a `profiles` (§6 del plan).

## Consecuencias
- Menos código y sin conflictos con la lógica de Supabase Auth.
- Si llega social login (federación), la normalización IDN se evalúa entonces — los emails siguen llegando normalizados a `profiles`.