-- 0002_profiles_trigger.sql — espejo al confirmar email (T003; FR-004)
-- Mecanismo elegido en research R11: trigger de DB sobre auth.users (no
-- webhook) — testeable en Supabase local. Crea el espejo en el momento de
-- la confirmación con email normalizado y placeholder "User" (Q1).

create or replace function create_profile_on_confirm() returns trigger as $$
begin
  -- Espejo 1:1 por cuenta; sin lógica extra: la API llena el resto (FR-004).
  insert into public.profiles (id, email, full_name)
  values (new.id, lower(trim(new.email)), 'User')
  on conflict (id) do nothing;

  return new;
end;
$$ language plpgsql security definer;

create trigger on_email_confirmed
  after update of email_confirmed_at on auth.users
  for each row
  when (new.email_confirmed_at is not null)
  execute function create_profile_on_confirm();