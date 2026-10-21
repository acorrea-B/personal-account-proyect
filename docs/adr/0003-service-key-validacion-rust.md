# ADR 0003: Service key + validación en Rust (RLS desactivada)

**Estado:** Aceptada — 2026-10-21 (D3)

## Contexto
El plan mencionaba RLS como mecanismo de aislamiento, pero es redundante si las funciones usan la `service_role` key (que la bypasea). Había que elegir el modelo de acceso a datos.

## Opciones
- **A. Anon key + RLS activo**: doble defensa (DB filtra por política aunque el handler tenga un bug); pero lógica duplicada (SQL policies + Rust) y policies difíciles de testear.
- **B. Service key + validación en Rust**: toda la lógica de acceso en un solo sitio; queries simples; `service_role` key confinada en secrets de Scaleway.

## Decisión
Opción **B**. Middleware valida JWT → extrae `user_id` → cada query del handler filtra por él. RLS desactivada (o deny-all por defecto como red de seguridad).

## Consecuencias
- Un bug de handler puede exponer datos y RLS no lo salvaría → compensado con **tests de acceso cruzado BOLA obligatorios por endpoint** (plan §11, API1:2023).
- La `service_role` key vive exclusivamente en Scaleway Secret Manager; nunca sale de la función ni llega al cliente.