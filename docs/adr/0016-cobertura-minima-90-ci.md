# ADR 0016: Cobertura mínima 90% como gate de CI

**Estado:** Aceptada — 2026-10-21 (D17)

## Contexto
Faltaba fijar la barra de calidad de testing para llegar a `main`. El plan §12.3 definía capas de testing (unitarias, integración, matriz de seguridad), pero cobertura como requisito duro no existía.

## Opciones
- **A. Sin umbral de cobertura**: los tests que el PR decida; la deuda de tests crece silenciosamente.
- **B. Cobertura mínima 90%** (líneas, unitarias + integración) como gate obligatorio de merge a `main`, con ratchet (no puede bajar respecto a `main`) y exclusiones justificadas por PR.

## Decisión
Opción **B**:
- El gate corre **unitarias + integración** con medición de cobertura de líneas, umbral global **≥90%**.
- Ratchet: si `main` sube el porcentaje, ningún PR puede bajarlo.
- Exclusiones (deben justificarse en el PR): código generado por macros externas, bootstrap (`main.rs`, config) — no representan lógica de negocio.
- Herramienta: `cargo-llvm-cov` como candidata (decisión fina del plan de implementación, donde se elige y se configura en el workflow).
- La matriz de seguridad obligatoria (plan §12.3) sigue siendo criterio independiente: cobertura ≠ corrección de seguridad.

## Consecuencias
- Ninguna lógica de negocio llega a `main` bajo 90%: los handlers auth/protected quedan cubiertos desde el primer PR.
- El ratchet evita el efecto valle: cada PR mantiene o mejora la barra.
- Riesgo: PRs de refactoring puro pueden sentir el gate — aceptado a cambio de la calidad global; las exclusiones con justificación son la válvula documentada.