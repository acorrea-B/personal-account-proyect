-- 0001_schema.sql — esquema inicial (T003; plan §6 / data-model.md)
-- Se ejecuta con la conexión DIRECTA de Supabase (MIGRATIONS_DATABASE_URL:
-- el pooler en modo transacción no soporta el advisory lock de sqlx, R10).

-- === Seguridad (D3) ===
-- Las funciones corren con service_role; RLS deny-all como red de seguridad:
-- sin políticas = ninguna conexión anónima puede leer. El aislamiento de
-- tenancy vive en Rust (filtro user_id en cada query, D3/FR-019).

-- === Perfil profesional (espejo de auth.users; normalización propia, D4) ===
create table profiles (
  id          uuid primary key references auth.users(id) on delete cascade,
  email       text not null,                    -- espejo normalizado (lowercase + trim)
  full_name   text not null,                    -- 1..200 tras trim (Rust, R3); placeholder "User" (Q1)
  headline    text,                             -- título profesional corto (≤120, R3)
  bio         text,                             -- (≤2000, R3)
  location    text,                             -- (≤120, R3)
  links       jsonb,                            -- [{"type":"linkedin","url":"..."}] — validación en Rust
  skills      jsonb,                            -- ["rust","aws",...] — validación en Rust
  processing_consent boolean,                   -- consent. para futura IA: null=nunca preguntado · false=rechazó · true=aceptó (D15)
  created_at  timestamptz not null default now(),
  updated_at  timestamptz not null default now()
);

-- Unicidad case-insensitiva del email (FR-002) + índice de login.
create unique index profiles_email_unique on profiles (lower(email));

-- === CV: versión actual (1 fila por usuario, ADR 0013) ===
create table cv_texts (
  id          uuid primary key default gen_random_uuid(),
  user_id     uuid not null unique references auth.users(id) on delete cascade,
  -- 50KB (FR-015): los BYTES los verifica el handler fail-fast sobre UTF-8;
  -- esta CHECK de backstop mide bytes sobre la conexión directa.
  content     text not null check (octet_length(content) <= 51200),
  created_at  timestamptz not null default now(),
  updated_at  timestamptz not null default now()
);

-- === CV: historial archivado (D13) ===
create table cv_versions (
  id          uuid primary key default gen_random_uuid(),
  user_id     uuid not null references auth.users(id) on delete cascade,
  content     text not null check (octet_length(content) <= 51200),
  origin      text not null default 'user' check (origin in ('user', 'ai')),  -- procedencia (D15): 'ai' solo post-beta
  archived_at timestamptz not null default now()
);

-- Listado paginado implícito del historial (≤10 vivas por retención).
create index cv_versions_user_recent on cv_versions (user_id, archived_at desc);

-- === updated_at por trigger (data-model: profiles y cv_texts) ===
create or replace function set_updated_at() returns trigger as $$
begin
  new.updated_at = now();
  return new;
end;
$$ language plpgsql;

create trigger profiles_updated_at
  before update on profiles
  for each row execute function set_updated_at();

create trigger cv_texts_updated_at
  before update on cv_texts
  for each row execute function set_updated_at();

-- === RLS enable (denegación por defecto en las tres tablas) ===
alter table profiles    enable row level security; -- sin políticas = deny-all
alter table cv_texts    enable row level security; -- sin políticas = deny-all
alter table cv_versions enable row level security; -- sin políticas = deny-all