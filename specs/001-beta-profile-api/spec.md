# Feature Specification: Beta API de Perfil Profesional

**Feature Branch**: `001-beta-profile-api`

**Created**: 2026-10-21

**Status**: Draft

**Input**: User description: "Sistema de autenticación y gestión de perfil profesional
para beta abierta: registro con verificación de email obligatoria, sesiones con tokens
renovables y revocables, CRUD de perfil profesional y CV como texto con historial de
versiones. Decisiones D1-D15 en docs/plan.md y docs/adr/."

## User Scenarios & Testing *(mandatory)*

### User Story 1 - Registro con verificación de email (Priority: P1)

Un visitante crea una cuenta con su email y una contraseña. El sistema le informa que
debe verificar su email; recibe un mensaje con un enlace de verificación. Hasta que
confirma, no puede usar el producto. Si no recibe el mensaje, puede solicitar otro.

**Why this priority**: Sin cuentas no existe el producto; la verificación garantiza
que los emails de la beta son reales y establece la identidad única de cada usuario.

**Independent Test**: Registrarse con un email válido, confirmar mediante el enlace
recibido, e intentar iniciar sesión — el flujo completo es verificable sin ninguna
otra funcionalidad del sistema.

**Acceptance Scenarios**:

1. **Given** un email válido y una contraseña que cumple los requisitos, **When** el
   usuario se registra, **Then** recibe respuesta de éxito (con los datos del usuario,
   sin credenciales) y un email de verificación.
2. **Given** un email ya registrado (aunque esté escrito con mayúsculas o espacios),
   **When** el usuario intenta registrarse de nuevo, **Then** recibe un aviso de
   conflicto que no revela información adicional de la cuenta existente.
3. **Given** una cuenta sin verificar, **When** el usuario intenta iniciar sesión,
   **Then** recibe un aviso claro de email no verificado con la opción explícita de
   reenviar la verificación.
4. **Given** la misma persona pide reenvíos repetidos, **When** supera el límite
   permitido, **Then** recibe un aviso de exceso de solicitudes (anti abuso).

### User Story 2 - Inicio de sesión y sesiones renovables/revocables (Priority: P2)

Un usuario verificado inicia sesión con su email y contraseña y obtiene una sesión
que puede extender sin volver a escribir su contraseña, y que puede revocar
(ej. al cerrar sesión en un dispositivo).

**Why this priority**: Es la puerta de uso del producto y sostiene todas las demás
funcionalidades; sin sesiones no hay interacciones autenticadas.

**Independent Test**: Con una cuenta ya verificada, iniciar sesión, renovar la sesión
antes de expirar y revocarla — verificable de punta a punta sin perfil ni CV.

**Acceptance Scenarios**:

1. **Given** una cuenta verificada con credenciales correctas, **When** el usuario
   inicia sesión, **Then** recibe credenciales de sesión y su identidad.
2. **Given** credenciales incorrectas o email inexistente, **When** se intenta
   iniciar sesión, **Then** el mensaje es idéntico en ambos casos (no revela si el
   email existe).
3. **Given** una sesión válida próxima a expirar, **When** el cliente usa su
   credencial de renovación, **Then** obtiene una nueva credencial y la anterior deja
   de ser utilizable (revocación por uso reincidente también falla).
4. **Given** una sesión activa, **When** el usuario la revoca, **Then** las
   siguientes peticiones con esa credencial son rechazadas.

### User Story 3 - Perfil profesional (Priority: P3)

Un usuario autenticado consulta y actualiza su perfil profesional: nombre completo,
título corto, biografía, ubicación, enlaces externos (ej. LinkedIn) y habilidades.

**Why this priority**: Es el contenido central del producto; requiere ya tener
cuenta y sesión (P1, P2) y desbloquea el CV como parte del mismo perfil.

**Independent Test**: Iniciar sesión, leer el perfil (vacío/al menos el nombre),
actualizarlo con datos válidos y verificar que la lectura refleja el cambio.

**Acceptance Scenarios**:

1. **Given** un usuario autenticado sin perfil completado, **When** consulta su
   perfil, **Then** obtiene un perfil inicial consistente (al menos nombre y email
   confirmado) y sin datos de otros usuarios.
2. **Given** datos válidos de perfil, **When** el usuario los guarda, **Then** una
   lectura posterior los devuelve sin alteración.
3. **Given** datos inválidos (nombre vacío, enlace con formato inválido,
   habilidades fuera del formato esperado), **When** el usuario los guarda,
   **Then** recibe un aviso específico por campo afectado y nada se guarda.
4. **Given** dos usuarios distintos, **When** uno consulta su perfil, **Then** solo
   ve el propio — ningún endpoint expone perfiles ajenos.

### User Story 4 - CV como texto con historial (Priority: P4)

Un usuario autenticado guarda el texto de su CV (máximo 50 KB), lo lee, lo reemplaza
y consulta su historial: cada reemplazo archiva la versión anterior, se conservan
las últimas 10 versiones y luego se purgan las más viejas.

**Why this priority**: Es el insumo de los futuros procesos avanzados (LLM); vale
independiente hoy como respaldo y editor de CV. Es la última pieza de la beta porque
está construida sobre cuenta, sesión y perfil.

**Independent Test**: Iniciar sesión, guardar un texto, reemplazarlo y consultar el
historial — verificable sin las decisiones de presentación del perfil.

**Acceptance Scenarios**:

1. **Given** un usuario sin CV guardado, **When** sube un texto válido (≤50 KB,
   UTF-8), **Then** recibe confirmación de creación con identificación de la versión.
2. **Given** un texto mayor al límite o con contenido inválido, **When** se intenta
   guardar, **Then** se rechaza con un aviso específico del motivo y nada se
   archiva.
3. **Given** un CV ya guardado, **When** el usuario reemplaza su texto, **Then** la
   versión anterior aparece en el historial etiquetada como de origen "usuario" y la
   actual refleja el nuevo contenido.
4. **Given** un historial que alcanza el límite de versiones, **When** se archiva
   una nueva versión, **Then** la más vieja sale de la lista y el usuario conserva
   exactamente las últimas 10.
5. **Given** un usuario autenticado, **When** intenta leer versiones de otro
   usuario, **Then** no existe forma de hacerlo por ningún parámetro de la petición.

### Edge Cases

- Email con espacios, mayúsculas o mayúscula inicial → se normaliza antes de
  cualquier decisión de unicidad.
- Petición con credencial expirada → motivo claro de expiración; el cliente puede
  renovar sin re-login.
- Petición con credencial de una sesión ya revocada → rechazo claro.
- Credencial malformada → rechazo como petición inválida (no como error interno).
- Cuerpo CV con bytes no válidos UTF-8 → rechazo con motivo explícito.
- CV de exactamente 50 KB → aceptado; 50 KB + 1 byte → rechazado.
- Escritura concurrente del mismo CV/perfil → la llegada tarde se rechaza con
  412 `stale-version` (nada se archiva ni sobrescribe).
- Dependencia de verificación de email indisponible al registrar → la cuenta queda
  creada; el usuario puede reenviar verificación cuando el servicio se recupera.
- Base de datos sin respuesta → respuesta de servicio no disponible con reintento
  seguro; ninguna operación queda a medias (guardar CV es todo-o-nada dentro de la
  transacción de archivado).
- Números de peticiones abusivas (login fallido repetido, reenvíos) → bloqueo
  temporal con aviso de reintento.

## Requirements *(mandatory)*

### Functional Requirements

**Registro y verificación**
- **FR-001**: El sistema MUST permitir registro con email + contraseña, rechazando
  emails malformados o contraseñas que no cumplan los requisitos publicados, con
  aviso específico del motivo.
- **FR-002**: El sistema MUST normalizar emails (minusculas, sin espacios) antes de
  cualquier decisión de unicidad; dos emails que solo difieren en mayúsculas son el
  mismo usuario.
- **FR-003**: El sistema MUST exigir verificación de email: la respuesta de registro
  contiene los datos del usuario pero NUNCA credenciales de sesión.
- **FR-004**: El sistema MUST crear el perfil profesional del usuario en el momento
  de la verificación de email, garantizando exactamente un perfil por cuenta; el
  `full_name` obligatorio se inicializa con el placeholder genérico "User" hasta
  que el usuario lo actualice (ver Clarifications).
- **FR-005**: El sistema MUST ofrecer reenvío de verificación con límites por
  usuario y IP para impedir abuso del envío de mensajes.
- **FR-006**: El sistema MUST responder a credenciales incorrectas con un mensaje
  idéntico al caso de email inexistente (anti enumeración) y con tiempos de
  respuesta comparables.

**Sesiones**
- **FR-007**: El sistema MUST emitir sesiones renovables: el usuario puede extender
  su sesión sin re-login mientras su credencial de renovación sea válida.
- **FR-008**: Cada credencial de renovación MUST ser de un solo uso: usada, se
  invalida y se emite una nueva; un intento de reuso MUST invalidar toda la sesión
  asociada (prevención de robo).
- **FR-009**: El sistema MUST permitir revocar la sesión y MUST rechazar cualquier
  petición posterior con su credencial.
- **FR-010**: Toda petición a endpoints protegidos MUST presentar una credencial
  válida, no expirada y de emisora/audiencia correctas; el sistema MUST rechazar con
  motivo explícito (no autenticado / expirado / revocado / inválido).
- **FR-011**: Los endpoints públicos (registro, login, reenvío, renovación) MUST ser
  accesibles sin credencial; todos los demás MUST estar protegidos (deny-by-default).

**Perfil**
- **FR-012**: El usuario autenticado MUST poder consultar y actualizar su perfil
  (nombre completo obligatorio; título, biografía, ubicación, enlaces y habilidades
  opcionales).
- **FR-013**: Las respuestas de perfil MUST incluir solo los campos del contrato
  público — nunca datos internos de la identidad ni metadata de sistema.
- **FR-014**: Enlaces y habilidades MUST validarse estructuralmente al guardar
  (formato de cada enlace, tipo de dato de cada habilidad) con aviso por campo
  afectado.

**CV**
- **FR-015**: El usuario autenticado MUST poder guardar, leer y reemplazar el texto
  de su CV, con límite de 50 KB y validación UTF-8; a 50 KB exacto se acepta, por
  encima se rechaza con aviso específico.
- **FR-016**: Cada reemplazo de CV MUST archivar el contenido anterior con etiqueta
  de procedencia (origen "usuario" en la beta) en una única operación atómica, junto
  con la actualización de la versión vigente. La escritura es **opcimista**: si el
  recurso cambió desde la última lectura del cliente (version = `updated_at`), se
  rechaza con `stale-version` (412) y nada se archiva ni sobrescribe (ver
  Clarifications).
- **FR-017**: El historial MUST conservar las últimas 10 versiones por usuario y
  MUST purgar las más viejas como parte de la misma operación de archivado (nunca
  crece sin límite).
- **FR-018**: El usuario autenticado MUST poder listar las versiones archivadas
  (metadatos: fecha y procedencia) y leer la versión completa de cualquiera de las
  suyas.
- **FR-019**: Ningún endpoint MUST permitir, por parámetro alguno, leer o escribir
  objetos (perfil, CV, versiones) de otro usuario.

**Contrato general**
- **FR-020**: Todos los errores de la API MUST seguir un formato estándar y único de
  "problema" con: tipo (identificable por slug), título, estado HTTP, descripción
  comprensible y ruta solicitada — con extensiones documentadas cuando aplica (ej.
  "se puede reenviar verificación").
- **FR-021**: El sistema MUST rate-limitar por IP y por usuario los flujos
  sensibles: login fallido, reenvío de verificación y escritura de CV; los estados
  429 acompañados del tiempo de espera razonable.
- **FR-022**: Los logs MUST registrar eventos con identificador de petición correlacionable,
  sin contraseñas, credenciales ni datos personales completos. Todo evento significativo
  del sistema (registro, verificación, login exitoso/fallido, sesiones, perfil, CV,
  bloqueos) MUST registrarse bajo un esquema y una taxonomía de nombres estándar y
  cerrados, correlacionable con la petición que lo originó.
- **FR-023**: Durante la beta, las solicitudes de eliminación de cuenta se atienden
  por el equipo (baja real de todos los datos del usuario); el esquema de datos ya
  garantiza la baja en cascada.

### Key Entities *(include if feature involves data)*

- **Cuenta (identidad)**: email único normalizado, estado verificado/no verificado,
  estado habilitado. Cada cuenta puede tener exactamente un perfil y un CV.
- **Perfil profesional**: nombre completo (obligatorio), título, biografía,
  ubicación, lista de enlaces (tipo + URL) y habilidades; consentimiento de
  procesamiento (sin usar en la beta, reservado para eras futuras).
- **CV**: una versión vigente por usuario (texto ≤ 50 KB) e historial de versiones
  archivadas (contenido, fecha de archivado, procedencia "usuario"/"IA", máx 10
  vigentes después de cada purga).
- **Sesión**: credencial de uso + credencial de renovación; única, renovable una vez
  por credencial, revocable.

## Success Criteria *(mandatory)*

### Measurable Outcomes

- **SC-001**: Un usuario nuevo pasa de registro a perfil completo en menos de 5
  minutos (incluida la verificación de email por enlace).
- **SC-002**: Las operaciones habituales (login, lectura/escritura de perfil y CV)
  completan en menos de 200 ms para el 95% de las peticiones, medido en entorno de
  staging con usuarios reales de pruebas.
- **SC-003**: En la matriz de pruebas de seguridad, 0 de los casos de "acceso
  cruzado" (un usuario leyendo/escribiendo objetos de otro por cualquier parámetro)
  tienen éxito.
- **SC-004**: En las pruebas del contrato de errores, 100% de respuestas de error
  siguen el formato estándar y ningún caso de credencial incorrecta / email
  inexistente produce mensajes o tiempos distinguibles entre sí.
- **SC-005**: Después de 40 reemplazos de CV por un mismo usuario, el historial
  contiene exactamente 10 versiones y ninguna fuera de retención.
- **SC-006**: El costo mensual del servicio en beta se mantiene bajo el presupuesto
  objetivo (€5/mes) monitorizando consumo de la plataforma.
- **SC-007**: De las peticiones de beta testers encuestadas para legibilidad de
  errores, ≥90% entienden qué corregir sin ayuda adicional.

## Assumptions

- Beta dirigida a un público pequeño/modesto (≤ 1k usuarios) antes de decisiones de
  escalado; los límites actuales (10 versiones, 50 KB, rate limits) se fijan como
  valores iniciales y son ajustables por configuración.
- Idioma de los mensajes de usuario: inglés (los payloads de la API —`title`,
  `detail` y valores de extensión— van en inglés, igual que el código; la
  documentación del proyecto permanece en español); el contrato de errores es
  agnóstico del idioma (tipos + status).

## Clarifications

### Session 2026-10-21

- Q: ¿Qué valor inicial tiene el campo obligatorio `full_name` al crear el perfil
  automáticamente en la verificación de email (FR-004)? → A: Placeholder genérico
  fijo "User" (idéntico para todos) hasta que el usuario lo actualice en
  `PUT /v1/profile`; mantiene la restricción no-nula sin inventar datos.
- Q: Si dos peticiones autenticadas del mismo usuario escriben su CV o perfil de
  forma casi simultánea, ¿cómo se resuelve el conflicto? → A: Concurrency optimista
  con condición del cliente: la escritura se rechaza (412, `stale-version`) si el
  recurso cambió desde la última lectura del cliente (version = `updated_at`
  devuelta por las lecturas); nada se archiva ni sobrescribe en conflicto. Aplica a
  `POST /v1/cv/text` y `PUT /v1/profile`.
- Eliminación de cuentas durante la beta: gestionada manualmente por solicitud (el
  endpoint de autoservicio es post-beta, D12/ADR 0012).
- La verificación de email obligatoria puede relajarse en v2 sin migración (D1/ADR
  0001) — asumido como aceptado el drop-off de beta.
- Procesos con LLM (`cv:process`) quedan explícitamente FUERA del alcance de esta
  feature; el consentimiento y la procedencia se recogen desde el primer dato, pero
  no se activan (D15/ADR 0014).