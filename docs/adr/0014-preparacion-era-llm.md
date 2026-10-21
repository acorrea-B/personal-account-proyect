# ADR 0014: Preparación para la era LLM

**Estado:** Aceptada — 2026-10-21 (D15)

## Contexto
El producto evolucionará hacia el uso de LLMs para pulir CVs y procesos avanzados para profesionales. La base v1 debía evaluarse: ¿qué conviene reservar ahora vs. diferir?

La base v1 ya es favorable a LLMs: CV como texto limpio (D7), perfil estructurado con `skills`/`links` jsonb (D14), historial `cv_versions` para pares antes/después (D13), middleware de rate limiting reutilizable, y ADR 0007 ya reserva el patrón 202+cola para `cv:process`.

## Opciones
- **A. Reservar ahora columnas de procedencia y consentimiento**: `cv_versions.origin` y `profiles.processing_consent`, vacías/sin lógica. Cero costo en v1; evita migración y **ambigüedad histórica** cuando llegue la IA.
- **B. Diferir todo**: menos columnas hoy; pero los datos históricos quedarían sin etiqueta de origen y el consentimiento GDPR habría que reconstruirlo retroactivamente.

## Decisión
Opción **A**. Al esquema v1 se agregan **sin lógica**:
- `cv_versions.origin text not null default 'user'` (`'user' | 'ai'`) — procedencia de cada versión.
- `profiles.processing_consent boolean` (`null` = nunca preguntado, `false` = rechazó, `true` = aceptó) — requisito GDPR previo a enviar CV a un proveedor LLM externo.

Se difieren a post-beta (plan §15): quota/credits por usuario para costos por token, cola de jobs asíncronos para `cv:process`, embeddings vía pgvector.

## Consecuencias
- Dos columnas, cero líneas de código en v1 (`cv:process` las exige antes de habilitarse).
- El historial queda auditado desde el primer dato: qué editó el usuario y qué la IA.
- El consentimiento se puede preguntar justo antes de habilitar procesamiento, cumpliendo GDPR desde el primer uso de IA.