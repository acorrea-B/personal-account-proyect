# ADR 0001: Verificación de email obligatoria

**Estado:** Aceptada — 2026-10-21 (D1)

## Contexto
El plan original tenía una contradicción: el flujo de registro emitía tokens inmediatos (3.1), pero los edge cases contemplaban "cuenta no verificada → 403" (4.2). Con la verificación de email activa en Supabase Auth, no se emiten tokens hasta confirmar.

## Opciones
- **A. Tokens inmediatos** con verificación suave (recordatorio): mínima fricción para beta abierta, permite usuarios con email falso.
- **B. Verificación obligatoria**: registro devuelve solo el usuario; login antes de confirmar falla. Menos usuarios fantasma, más fricción (drop-off si el email tarda).

## Decisión
Opción **B**. Registro → `201 { user }` sin tokens. Login sin confirmar → `403 { error: "email_not_verified", can_resend: true }`. Nuevo endpoint `POST /auth/resend-verification` con rate limiting estricto.

## Consecuencias
- Supabase Auth lo soporta nativamente (signup sin sesión, login pre-confirmación → `email_not_confirmed`): sin código extra.
- Drop-off aceptado como trade-off de beta; relajable en v2 sin migración.