# ADR 0013: Historial del CV en tabla `cv_versions` (retención 10)

**Estado:** Aceptada — 2026-10-21 (D13)

## Contexto
La tabla de riesgos del plan exige "backups automáticos, versionado" para pérdida de CV. Había que decidir si el versionado vive solo en backups o también en la app.

## Opciones
- **A. Solo versión actual + backups pg_dump**: tabla única, handler simple; sin historial consultable desde la app.
- **B. Historial en `cv_versions`**: cada `PUT` archiva el contenido anterior; recuperación posible desde la app.

## Decisión
Opción **B**, con reglas:
- `cv_texts` sigue siendo 1 fila por usuario (upsert); `cv_versions` crece solo por ediciones.
- Cada `PUT /cv/text` corre en **una transacción**: archiva el anterior (si existía) → upsert de la versión actual → purga versiones fuera de retención.
- **Retención: últimas 10 versiones** por usuario (purgadas en la misma transacción, nunca una tarea aparte).
- Índice `(user_id, archived_at desc)` para listado por recencia.

## Consecuencias
- Escrituras de CV son ligeramente más caras (3 operaciones en vez de 1) — aceptable a volumen beta.
- La purga transaccional garantiza que `cv_versions` nunca crece sin límite.
- Los backups pg_dump siguen operando como última línea de defensa ante desastres, complementando (no substituyendo) el historial.