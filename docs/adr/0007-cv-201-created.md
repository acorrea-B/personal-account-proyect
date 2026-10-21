# ADR 0007: `201 Created` para `POST /cv/text`

**Estado:** Aceptada — 2026-10-21 (D5)

## Contexto
El plan original respondía `202 Accepted`, que implica procesamiento asíncrono en cola. Pero el flujo de v1 es síncrono: validar, guardar, responder.

## Opciones
- **A. Mantener 202**: semánticamente incorrecto (promete confirmación futura que no existe), engaña a clientes y métricas.
- **B. 201 Created**: el recurso se creó y está disponible inmediatamente.

## Decisión
Opción **B**: `201 { cv_id, updated_at }` (upsert de la fila única por usuario).

## Consecuencias
- Contrato HTTP correcto y simple; si `cv:process` (v2) introduce cola, ese flujo tendrá su propio endpoint con 202/202+webhook, sin tocar este.