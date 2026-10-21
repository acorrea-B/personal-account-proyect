# ADR 0010: Trunk-based + PR con reglas de granularidad

**Estado:** Aceptada — 2026-10-21 (D10)

## Contexto
Faltaba definir cómo entra el código a `main` (equipo pequeño, beta iterando rápido).

## Opciones
- **A. Trunk-based + PR**
- **B. GitFlow** (`develop`/`release`/`hotfix`): más formalidad pensada para equipos grandes; overhead y merge conflict fatigue a esta escala.

## Decisión
Opción **A**, con reglas propias:

1. `main` siempre deployable; ramas cortas `feature/*` → PR con CI (fmt, clippy, tests, audit) → squash-merge. Conventional Commits.
2. **Máx 6 archivos por rama/PR.** Si una feature excede, se sub-divide en ramas apiladas (ej: CRUD de 9 archivos → "CR" 5 archivos → PR → revisión → merge → "UD" 4 archivos → PR).
3. **Pausa obligatoria en cada rama** que cumpla la condición: revisión manual iterativa del proyecto antes de continuar.
4. **Máx 2 structs/clases por archivo**, salvo centralizaciones de utils o enums.

## Consecuencias
- Revisión en trozos pequeños y predecibles; riesgo controlado de PRs gigantes.
- Las ramas apiladas requieren disciplina de rebase; si el overhead se vuelve fricción, se ajusta este ADR (no se rompe la regla silenciosamente).