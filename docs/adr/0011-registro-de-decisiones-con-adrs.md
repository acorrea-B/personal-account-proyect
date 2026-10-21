# ADR 0011: Registro de decisiones mediante ADRs

**Estado:** Aceptada — 2026-10-21 (D11)

## Contexto
Al iniciar el proyecto, las decisiones de arquitectura solo vivían en el plan y en conversaciones; nada respondía "¿por qué hicimos esto así?" de forma auditable en el repo.

## Opciones
- **A. ADRs** en `docs/adr/`: documento corto por decisión significativa.
- **B. Solo plan.md + git history**: cero mantenimiento; el contexto queda atrapado en el chat.

## Decisión
Opción **A**. Formato: **Estado / Contexto / Opciones / Decisión / Consecuencias**. Se crea un ADR cuando una decisión es difícil de revertir o afecta contrato público. Numeración secuencial; los ADRs son inmutables — las correcciones son nuevos ADRs que marcan a los anteriores como superseded.

## Consecuencias
- Este mismo registro fue aplicado retroactivamente: ADR 0001–0010 espejan las decisiones D1–D10 tomadas durante la iteración del plan (2026-10-21).
- `docs/plan.md` queda como mapa del proyecto; los ADRs documentan el *por qué*.