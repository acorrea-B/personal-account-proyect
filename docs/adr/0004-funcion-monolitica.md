# ADR 0004: Una sola función monolítica `api`

**Estado:** Aceptada — 2026-10-21 (D6)

## Contexto
"Despliegue independiente por función" era ambiguo: cada función Scaleway es un contenedor con cold start propio. Con ~9 rutas, había que decidir granularidad.

## Opciones
- **A. 1 función monolítica** con router interno.
- **B. 2 funciones** (`auth` + `app`): aísla el código sensible en secret management más estricto y permite deploys por dominio; dos deployments, dos cold starts, middleware duplicado o crate compartido.

## Decisión
Opción **A**: una función `api` que expone `/v1/auth/*`, `/v1/profile` y `/v1/cv/*`. Un deployment, un cold start por contenedor, middleware y DB pool compartidos naturalmente.

## Consecuencias
- Simplicidad operativa (un pipeline, un namespace) y menor costo inicial.
- Si un dominio crece, la extracción a N funciones en v2 es aditiva: el crate ya separa `domain/` por dominio.