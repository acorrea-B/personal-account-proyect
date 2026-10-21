# ADR 0008: Framework Rust — axum + tokio

**Estado:** Aceptada — 2026-10-21 (D8)

## Contexto
La función `api` necesita router, middleware y extracción de datos HTTP. La selección afecta DRY (middleware compartido), cold start y mantenibilidad.

## Opciones
- **A. axum + tokio**: middleware en capas sobre tower (composable — ideal para el gate JWT compartido), ecosistema dominante, sin acoplamiento a runtime propietario.
- **B. actix-web**: madurez y rendimiento crudo similar/mayor, pero ecosistema más fragmentado; sin beneficio real al volumen de beta.
- **C. Sin framework** (runtime crudo de Scaleway): cold start mínimo, pero router/extractors/middleware a mano — anti-DRY, riesgo de bugs.

## Decisión
Opción **A**. Stack: axum + tower/tower-http + tokio (detalle en plan §13.1).

## Consecuencias
- El gate JWT, rate limiting, timeouts y logging se aplican capa por capa vía tower y son compartidos por todas las rutas.
- Sin dependencia de runtime propietario de Scaleway: el binario es un servidor HTTP estándar en el contenedor.