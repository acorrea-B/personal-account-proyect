# ADR 0005: Texto del CV en PostgreSQL (columna TEXT)

**Estado:** Aceptada — 2026-10-21 (D7)

## Contexto
El plan original ponía el texto del CV en Supabase Storage con metadata en PostgreSQL. Para texto plano sin procesamiento (v1), se evaluó si Storage era necesario.

## Opciones
- **A. Storage + metadata en DB**: escala a binarios sin tocar backend; pero doble consulta, buckets, URLs firmadas y más piezas de seguridad para un caso simple.
- **B. Columna TEXT**: con 50KB/usuario, el plan gratuito (500MB) sobra. Transacciones ACID con el resto, backup con pg_dump, una sola query para CV, sin buckets ni URL firmadas.

## Decisión
Opción **B**: tabla `cv_texts` (`user_id` único, `content TEXT` con CHECK ≤ 51200 bytes).

## Consecuencias
- Si v2 agrega PDFs, Supabase Storage se incorpora de forma **aditiva** (sin migrar lo existente).
- Búsqueda/ordenamiento del texto con índices Postgres si algún día hace falta.