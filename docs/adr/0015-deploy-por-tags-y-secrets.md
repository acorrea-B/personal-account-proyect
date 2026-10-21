# ADR 0015: CI/CD con despliegue por tags y secretos de la función

**Estado:** Aceptada — 2026-10-21 (D16)

## Contexto
Faltaba definir cómo llega un release a producción y qué secretos necesita la función para funcionar. El plan §12.2 tenía la promoción como pregunta abierta.

## Opciones (promoción)
- **A. Deploy manual** desde la máquina del operador: simple, pero irreproducible y sin trazabilidad.
- **B. Deploy por tags**: un tag `vX.Y.Z` (semver, anotado) en GitHub dispara el deploy de producción vía GitHub Actions; staging se despliega automático con cada merge a `main`.

## Decisión
Opción **B** (detalle operativo en plan §12.2):
- PR → gates CI (fmt, clippy, tests, audit) · merge `main` → staging · **tag `vX.Y.Z` → producción**
- Producción corre tras un **GitHub Environment protegido** con aprobación manual; rollback = re-desplegar el tag anterior.
- Imágenes trazables: `api:{git-sha}` / `api:{v-tag}`.

**Secrets de la función** (en Scaleway Secret Manager, por entorno staging/producción):
`SERVICE_ROLE_KEY` · `SUPABASE_JWT_SECRET` (o JWKS URL) · `SUPABASE_URL` · `DATABASE_URL` (pooler) · `SUPABASE_WEBHOOK_SECRET` (opcional).

**Regla dura:** los secrets de la función nunca pasan por GitHub ni por el repo — el pipeline solo referencia los nombres; los valores viven exclusivamente en Scaleway Secret Manager. Credenciales de deploy (`SCW_ACCESS_KEY`/`SCW_SECRET_KEY`) sí viven en GitHub Environment secrets, con scope limitado y rotación.

## Consecuencias
- Releases reproducibles y revertibles por tag; el changelog es la release note del tag.
- El blast radius de una fuga de GitHub queda limitado a credenciales de deploy rotables, no a secretos de la aplicación (ni siquiera están ahí).
- Rotación de `service_role`/JWT secret cada 90 días vía Scaleway Secret Manager + Supabase, sin re-deploy.