# ADR 0002: Sin scopes OAuth2 en beta

**Estado:** Aceptada — 2026-10-21 (D2)

## Contexto
La matriz original contemplaba scopes granulares (`profile:read`, `cv:write`...), pero Supabase Auth no emite JWTs con claim `scope` por defecto. Además, con un único cliente (el propio frontend), la granularidad aporta poco en beta.

## Opciones
- **A. Custom Access Token Hook + enforcement de scopes ya**: máximo control desde el día 1, pero hook SQL + validación por endpoint a probar antes de tener usuarios.
- **B. Emitir tokens OAuth2 propios** ("OAuth2 completo"): scopes y revocación de verdad, pero se pierde login/signup/refresh gratis de Supabase — construir un auth server entero.
- **C. Sin scopes en beta**: tokens con todos los permisos del usuario; middleware valida JWT y basta.

## Decisión
Opción **C**. La matriz de endpoints pierde la columna "scope"; todo token válido da acceso. `auth:refresh`/`auth:revoke` salen de la matriz: son flujos, no permisos. La sección de scopes queda como **diseño futuro**.

## Consecuencias
- Middleware de beta queda simple: firma → exp → iss/aud → usuario existe.
- Activación futura vía Custom Access Token Hook (claim `scope`) + extensión del middleware: **sin migración de datos ni romper clientes**.