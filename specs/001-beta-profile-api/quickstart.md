# Quickstart: Beta API de Perfil Profesional

**Date**: 2026-10-21 · Guía de validación end-to-end (provee el "funciona" demostrable
post-implementación). Referencias: [contracts/](./contracts/), [data-model.md](./data-model.md).

## Prerrequisitos

- Rust estable (`rustup`), `docker` (corriendo), Supabase CLI, `python3`/un shell POSIX
- Opcional para deploy: `scw` CLI configurado (no se requiere para el flujo local)

## 1. Levantar el stack local

```bash
supabase init            # primera vez (o clona el repo: ya viene configurado)
supabase start           # auth + db + storage en variables locales que imprime
supabase status          # anota: API_URL, ANON_KEY, JWT_SECRET, DB_URL
```

## 2. Variables de entorno de la función

```bash
export DATABASE_URL="postgres://postgres:postgres@localhost:54322/postgres"
export SUPABASE_URL="http://localhost:54321"
export SUPABASE_JWT_SECRET="<JWT_SECRET del supabase status>"
export ENV=development
# rate limits/pool: defaults de config.rs (no secretas)
```
La `service_role` key solo se necesitará en staging/producción (Scaleway Secret
Manager, D16) — en local deriva de `supabase status`.

## 3. Migraciones + servidor

```bash
sqlx migrate run          # migra el esquema de data-model.md (profiles, cv_texts, cv_versions)
cargo run                 # arranca el servidor: http://localhost:8080/v1
```

## 4. Ejercicios de validación (contratos paso a paso)

**A. Auth — P1/P2 (contratos en http-auth.md):**
```bash
# 1. registro (201 + email en la bandeja de la UI de supabase local)
curl -s -X POST localhost:8080/v1/auth/register \
  -d '{"email":"  Juan@Example.com ","password":"contraseña1"}'
# resultado esperado: user con email normalizado, SIN tokens
# 2. verificar: confirmar por el email (o `supabase` admin local) → aparece profiles
# 3. login con el email normalizado → 200 con access_token/refresh_token
# 4. login sin verificar (otra cuenta nueva) → 403 email-not-verified + can_resend
# 5. refresh con el token viejo DOS veces: la 2ª → 401 refresh-token-reused
# 6. revoke con token vigente → 204; usar el mismo access_token → 401
```
**B. Perfil — P3 (http-profile.md):**
```bash
# con access_token de la sesión:
curl -s localhost:8080/v1/profile -H "Authorization: Bearer $TOKEN"
# → perfil inicial (email espejo, sin datos de terceros)
curl -s -X PUT localhost:8080/v1/profile -H "Authorization: Bearer $TOKEN" \
  -d '{"full_name":"Juan Perez","links":[{"type":"linkedin","url":"no-es-url"}]}'
# → 422 profile-invalid-field con fields: {"links": "..."} — y nada guardado
```
**C. CV y versiones — P4 (http-cv.md, SC-005):**
```bash
# 1. POST content 50 KB justo → 201; 51201 bytes → 413 cv-too-large
# 2. reemplazar 12 veces seguidas → GET /v1/cv/versions: exactamente 10 entradas
# 3. GET /v1/cv/versions/{id} de la más vieja aún presente → 200
#    con id FUERA del listado (vencida) → 404 not-found (sin revelar si existió)
```
**D. Seguridad — SC-003 (matriz de tests/security/):**
```bash
# con access_token del usuario B:
curl -s localhost:8080/v1/cv/text -H "Authorization: Bearer $TOKEN_B"
# → 404 (CV propio inexistente) — en NINGÚN parámetro se puede leer el CV de A.
#   Correr `cargo test --test bola` automatiza: cada ruta, usuario ajeno, 401/404 esperados.
```

## 5. Tests y cobertura (gates D17)

```bash
cargo test                      # unitarias + integración (supabase local levantado)
cargo llvm-cov --fail-under-lines 90    # gate de cobertura ≥90% (ADR 0016)
cargo clippy -- -D warnings && cargo fmt --check && cargo audit   # resto de gates
```

## Resultado esperado global

A/B/C/D: verde + `scc` de gates en verde = spec demostrado de punta a punta en local.
Para staging/producción: `.github/workflows/deploy.yml` (D16) — staging con merge a
main, producción por tag `vX.Y.Z` con secrets desde Scaleway Secret Manager.